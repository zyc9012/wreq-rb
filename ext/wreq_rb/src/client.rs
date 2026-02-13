use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::any::Any;
use std::time::Duration;

use magnus::{
    function, method, prelude::*, Module, RArray, RHash, Ruby,
    try_convert::TryConvert, Value,
};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;
use wreq::header::{HeaderMap, HeaderName, HeaderValue};
use wreq_util::Emulation as BrowserEmulation;

use crate::error::{generic_error, to_magnus_error};
use crate::response::Response;

// --------------------------------------------------------------------------
// Shared Tokio runtime
// --------------------------------------------------------------------------

fn runtime() -> &'static Runtime {
    use std::sync::OnceLock;
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
    })
}

// --------------------------------------------------------------------------
// GVL release helper
// --------------------------------------------------------------------------

/// Run a closure without the Ruby GVL, allowing other Ruby threads to execute.
/// The closure receives a `CancellationToken` that is cancelled if Ruby
/// interrupts the thread (e.g. `Thread.kill`, signal, timeout).
///
/// # Safety
/// The closure must NOT access any Ruby objects or call any Ruby C API.
/// Extract all data from Ruby before calling this, convert results after.
unsafe fn without_gvl<F, R>(f: F) -> R
where
    F: FnOnce(CancellationToken) -> R,
{
    struct CallData<F, R> {
        func: Option<F>,
        result: Option<R>,
        token: CancellationToken,
        panic_payload: Option<Box<dyn Any + Send>>,
    }

    unsafe extern "C" fn call<F, R>(data: *mut c_void) -> *mut c_void
    where
        F: FnOnce(CancellationToken) -> R,
    {
        let d = data as *mut CallData<F, R>;
        let f = (*d).func.take().unwrap();
        let token = (*d).token.clone();
        // catch_unwind prevents a panic from unwinding through C frames (UB).
        match panic::catch_unwind(AssertUnwindSafe(|| f(token))) {
            Ok(val) => ptr::write(&mut (*d).result, Some(val)),
            Err(payload) => (*d).panic_payload = Some(payload),
        }
        ptr::null_mut()
    }

    /// Unblock function called by Ruby when it wants to interrupt this thread.
    /// Cancels the token so the in-flight async work can abort promptly.
    unsafe extern "C" fn ubf<F, R>(data: *mut c_void) {
        let d = data as *const CallData<F, R>;
        (*d).token.cancel();
    }

    let mut data = CallData {
        func: Some(f),
        result: None,
        token: CancellationToken::new(),
        panic_payload: None,
    };
    let data_ptr = &mut data as *mut CallData<F, R> as *mut c_void;

    unsafe {
        rb_sys::rb_thread_call_without_gvl(
            Some(call::<F, R>),
            data_ptr,
            Some(ubf::<F, R>),
            data_ptr,
        );
    }

    if let Some(payload) = data.panic_payload {
        panic::resume_unwind(payload);
    }

    data.result.unwrap()
}

/// Collected response data as pure Rust types (no Ruby objects).
struct ResponseData {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    url: String,
    version: String,
    content_length: Option<u64>,
}

/// Outcome of the network call performed outside the GVL.
enum RequestOutcome {
    Ok(ResponseData),
    Err(wreq::Error),
    Interrupted,
}

/// Execute a request and collect the full response as pure Rust types.
async fn execute_request(req: wreq::RequestBuilder) -> Result<ResponseData, wreq::Error> {
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let url = resp.uri().to_string();
    let version = format!("{:?}", resp.version());
    let content_length = resp.content_length();
    let headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
        .collect();
    let body = resp.bytes().await?.to_vec();
    Ok(ResponseData { status, headers, body, url, version, content_length })
}

// --------------------------------------------------------------------------
// Emulation helpers
// --------------------------------------------------------------------------

/// The default emulation to apply when none is specified.
const DEFAULT_EMULATION: BrowserEmulation = BrowserEmulation::Chrome143;

/// Parse a Ruby string like "chrome_143" into a BrowserEmulation variant.
fn parse_emulation(name: &str) -> Result<BrowserEmulation, magnus::Error> {
    let json_val = serde_json::Value::String(name.to_string());
    serde_json::from_value::<BrowserEmulation>(json_val)
        .map_err(|_| generic_error(format!("unknown emulation: '{}'. Use names like 'chrome_143', 'firefox_146', 'safari_18_5', etc.", name)))
}

// --------------------------------------------------------------------------
// Ruby Client
// --------------------------------------------------------------------------

#[magnus::wrap(class = "Wreq::Client", free_immediately)]
struct Client {
    inner: wreq::Client,
}

impl Client {
    /// Wreq::Client.new or Wreq::Client.new(options_hash)
    fn rb_new(args: &[Value]) -> Result<Self, magnus::Error> {
        let opts: Option<RHash> = if args.is_empty() {
            None
        } else {
            Some(RHash::try_convert(args[0])?)
        };

        let mut builder = wreq::Client::builder();

        if let Some(opts) = opts {
            if let Some(val) = hash_get_value(&opts, "emulation")? {
                let ruby = unsafe { Ruby::get_unchecked() };
                if val.is_kind_of(ruby.class_false_class()) {
                    // emulation: false — skip emulation entirely
                } else if val.is_kind_of(ruby.class_true_class()) {
                    builder = builder.emulation(DEFAULT_EMULATION);
                } else {
                    let name: String = TryConvert::try_convert(val)?;
                    let emu = parse_emulation(&name)?;
                    builder = builder.emulation(emu);
                }
            } else {
                builder = builder.emulation(DEFAULT_EMULATION);
            }

            if let Some(ua) = hash_get_string(&opts, "user_agent")? {
                builder = builder.user_agent(ua);
            }

            if let Some(hdr_hash) = hash_get_hash(&opts, "headers")? {
                let hmap = hash_to_header_map(&hdr_hash)?;
                builder = builder.default_headers(hmap);
            }

            if let Some(t) = hash_get_float(&opts, "timeout")? {
                builder = builder.timeout(Duration::from_secs_f64(t));
            }

            if let Some(t) = hash_get_float(&opts, "connect_timeout")? {
                builder = builder.connect_timeout(Duration::from_secs_f64(t));
            }

            if let Some(t) = hash_get_float(&opts, "read_timeout")? {
                builder = builder.read_timeout(Duration::from_secs_f64(t));
            }

            if let Some(val) = hash_get_value(&opts, "redirect")? {
                let ruby = unsafe { Ruby::get_unchecked() };
                if val.is_kind_of(ruby.class_false_class()) {
                    builder = builder.redirect(wreq::redirect::Policy::none());
                } else if val.is_kind_of(ruby.class_true_class()) {
                    builder = builder.redirect(wreq::redirect::Policy::limited(10));
                } else {
                    let n: usize = TryConvert::try_convert(val)?;
                    builder = builder.redirect(wreq::redirect::Policy::limited(n));
                }
            }

            if let Some(enabled) = hash_get_bool(&opts, "cookies")? {
                builder = builder.cookie_store(enabled);
            }

            if let Some(proxy_url) = hash_get_string(&opts, "proxy")? {
                let mut proxy = wreq::Proxy::all(&proxy_url).map_err(to_magnus_error)?;
                if let (Some(user), Some(pass)) = (
                    hash_get_string(&opts, "proxy_user")?,
                    hash_get_string(&opts, "proxy_pass")?,
                ) {
                    proxy = proxy.basic_auth(&user, &pass);
                }
                builder = builder.proxy(proxy);
            }

            if let Some(true) = hash_get_bool(&opts, "no_proxy")? {
                builder = builder.no_proxy();
            }

            if let Some(enabled) = hash_get_bool(&opts, "https_only")? {
                builder = builder.https_only(enabled);
            }

            if let Some(true) = hash_get_bool(&opts, "http1_only")? {
                builder = builder.http1_only();
            }
            if let Some(true) = hash_get_bool(&opts, "http2_only")? {
                builder = builder.http2_only();
            }

            if let Some(v) = hash_get_bool(&opts, "gzip")? {
                builder = builder.gzip(v);
            }
            if let Some(v) = hash_get_bool(&opts, "brotli")? {
                builder = builder.brotli(v);
            }
            if let Some(v) = hash_get_bool(&opts, "deflate")? {
                builder = builder.deflate(v);
            }
            if let Some(v) = hash_get_bool(&opts, "zstd")? {
                builder = builder.zstd(v);
            }
        } else {
            builder = builder.emulation(DEFAULT_EMULATION);
        }

        let client = builder.build().map_err(to_magnus_error)?;
        Ok(Client { inner: client })
    }

    /// client.get(url) or client.get(url, opts)
    fn get(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("GET", args)
    }

    fn post(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("POST", args)
    }

    fn put(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("PUT", args)
    }

    fn patch(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("PATCH", args)
    }

    fn delete(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("DELETE", args)
    }

    fn head(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("HEAD", args)
    }

    fn options(&self, args: &[Value]) -> Result<Response, magnus::Error> {
        self.execute_method("OPTIONS", args)
    }

    fn execute_method(&self, method_str: &str, args: &[Value]) -> Result<Response, magnus::Error> {
        let url: String = if args.is_empty() {
            return Err(generic_error("url is required"));
        } else {
            TryConvert::try_convert(args[0])?
        };

        let opts: Option<RHash> = if args.len() > 1 {
            Some(RHash::try_convert(args[1])?)
        } else {
            None
        };

        let method: wreq::Method = method_str
            .parse()
            .map_err(|_| generic_error(format!("invalid HTTP method: {}", method_str)))?;

        let mut req = self.inner.request(method, &url);

        if let Some(opts) = opts {
            req = apply_request_options(req, &opts)?;
        }

        // Release the GVL so other Ruby threads can run during I/O.
        // All Ruby data has been extracted into Rust types above.
        // The closure receives a CancellationToken that is triggered if Ruby
        // wants to interrupt this thread (Thread.kill, signal, etc.).
        let outcome: RequestOutcome = unsafe {
            without_gvl(|cancel| {
                runtime().block_on(async {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => RequestOutcome::Interrupted,
                        res = execute_request(req) => match res {
                            Ok(data) => RequestOutcome::Ok(data),
                            Err(e) => RequestOutcome::Err(e),
                        },
                    }
                })
            })
        };

        let data = match outcome {
            RequestOutcome::Ok(d) => d,
            RequestOutcome::Err(e) => return Err(to_magnus_error(e)),
            RequestOutcome::Interrupted => return Err(generic_error("request interrupted")),
        };
        Ok(Response::new(data.status, data.headers, data.body, data.url, data.version, data.content_length))
    }
}

fn apply_request_options(
    mut req: wreq::RequestBuilder,
    opts: &RHash,
) -> Result<wreq::RequestBuilder, magnus::Error> {
    if let Some(hdr_hash) = hash_get_hash(opts, "headers")? {
        let hmap = hash_to_header_map(&hdr_hash)?;
        req = req.headers(hmap);
    }

    if let Some(body_str) = hash_get_string(opts, "body")? {
        req = req.body(body_str);
    }

    if let Some(json_val) = hash_get_value(opts, "json")? {
        let json_str = ruby_to_json_string(json_val)?;
        req = req
            .header("content-type", "application/json")
            .body(json_str);
    }

    if let Some(form_hash) = hash_get_hash(opts, "form")? {
        let pairs = hash_to_pairs(&form_hash)?;
        req = req.form(&pairs);
    }

    if let Some(query_hash) = hash_get_hash(opts, "query")? {
        let pairs = hash_to_pairs(&query_hash)?;
        req = req.query(&pairs);
    }

    if let Some(t) = hash_get_float(opts, "timeout")? {
        req = req.timeout(Duration::from_secs_f64(t));
    }

    if let Some(token) = hash_get_string(opts, "auth")? {
        req = req.auth(token);
    }

    if let Some(token) = hash_get_string(opts, "bearer")? {
        req = req.bearer_auth(token);
    }

    if let Some(basic_val) = hash_get_value(opts, "basic")? {
        let ary = RArray::try_convert(basic_val)?;
        if ary.len() >= 2 {
            let user: String = TryConvert::try_convert(ary.entry::<Value>(0)?)?;
            let pass: String = TryConvert::try_convert(ary.entry::<Value>(1)?)?;
            req = req.basic_auth(user, Some(pass));
        }
    }

    if let Some(proxy_url) = hash_get_string(opts, "proxy")? {
        let proxy = wreq::Proxy::all(&proxy_url).map_err(to_magnus_error)?;
        req = req.proxy(proxy);
    }

    if let Some(val) = hash_get_value(opts, "emulation")? {
        let ruby = unsafe { Ruby::get_unchecked() };
        if val.is_kind_of(ruby.class_false_class()) {
            // emulation: false — no per-request emulation override
        } else if val.is_kind_of(ruby.class_true_class()) {
            req = req.emulation(DEFAULT_EMULATION);
        } else {
            let name: String = TryConvert::try_convert(val)?;
            let emu = parse_emulation(&name)?;
            req = req.emulation(emu);
        }
    }

    Ok(req)
}

// --------------------------------------------------------------------------
// Module-level convenience methods
// --------------------------------------------------------------------------

fn wreq_get(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("GET", args)
}

fn wreq_post(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("POST", args)
}

fn wreq_put(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("PUT", args)
}

fn wreq_patch(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("PATCH", args)
}

fn wreq_delete(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("DELETE", args)
}

fn wreq_head(args: &[Value]) -> Result<Response, magnus::Error> {
    let client = Client::rb_new(&[])?;
    client.execute_method("HEAD", args)
}

// --------------------------------------------------------------------------
// Hash helpers
// --------------------------------------------------------------------------

fn hash_get_value(hash: &RHash, key: &str) -> Result<Option<Value>, magnus::Error> {
    // Try string key
    let val: Value = hash.aref(key)?;
    if !val.is_nil() {
        return Ok(Some(val));
    }
    // Try symbol key
    let ruby = unsafe { Ruby::get_unchecked() };
    let sym = ruby.to_symbol(key);
    let val: Value = hash.aref(sym)?;
    if !val.is_nil() {
        return Ok(Some(val));
    }
    Ok(None)
}

fn hash_get_string(hash: &RHash, key: &str) -> Result<Option<String>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(TryConvert::try_convert(v)?)),
        None => Ok(None),
    }
}

fn hash_get_float(hash: &RHash, key: &str) -> Result<Option<f64>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(TryConvert::try_convert(v)?)),
        None => Ok(None),
    }
}

fn hash_get_bool(hash: &RHash, key: &str) -> Result<Option<bool>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(TryConvert::try_convert(v)?)),
        None => Ok(None),
    }
}

fn hash_get_hash(hash: &RHash, key: &str) -> Result<Option<RHash>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(RHash::try_convert(v)?)),
        None => Ok(None),
    }
}

fn hash_to_header_map(hash: &RHash) -> Result<HeaderMap, magnus::Error> {
    let mut hmap = HeaderMap::new();
    hash.foreach(|k: String, v: String| {
        let name =
            HeaderName::from_bytes(k.as_bytes()).map_err(|e| generic_error(e))?;
        let value = HeaderValue::from_str(&v).map_err(|e| generic_error(e))?;
        hmap.insert(name, value);
        Ok(magnus::r_hash::ForEach::Continue)
    })?;
    Ok(hmap)
}

fn hash_to_pairs(hash: &RHash) -> Result<Vec<(String, String)>, magnus::Error> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    hash.foreach(|k: Value, v: Value| {
        let ruby = unsafe { Ruby::get_unchecked() };
        let ks: String = if k.is_kind_of(ruby.class_symbol()) {
            let s: String = k.funcall("to_s", ())?;
            s
        } else {
            TryConvert::try_convert(k)?
        };
        let vs: String = v.funcall("to_s", ())?;
        pairs.push((ks, vs));
        Ok(magnus::r_hash::ForEach::Continue)
    })?;
    Ok(pairs)
}

// --------------------------------------------------------------------------
// Ruby to JSON conversion
// --------------------------------------------------------------------------

fn ruby_to_json_string(val: Value) -> Result<String, magnus::Error> {
    let json_val = ruby_to_json(val)?;
    serde_json::to_string(&json_val).map_err(|e| generic_error(e))
}

fn ruby_to_json(val: Value) -> Result<serde_json::Value, magnus::Error> {
    let ruby = unsafe { Ruby::get_unchecked() };
    if val.is_nil() {
        return Ok(serde_json::Value::Null);
    }
    if val.is_kind_of(ruby.class_true_class()) {
        return Ok(serde_json::Value::Bool(true));
    }
    if val.is_kind_of(ruby.class_false_class()) {
        return Ok(serde_json::Value::Bool(false));
    }
    if val.is_kind_of(ruby.class_integer()) {
        let i: i64 = TryConvert::try_convert(val)?;
        return Ok(serde_json::Value::Number(i.into()));
    }
    if val.is_kind_of(ruby.class_float()) {
        let f: f64 = TryConvert::try_convert(val)?;
        return Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()),
        ));
    }
    if val.is_kind_of(ruby.class_string()) {
        let s: String = TryConvert::try_convert(val)?;
        return Ok(serde_json::Value::String(s));
    }
    if val.is_kind_of(ruby.class_array()) {
        let ary = RArray::try_convert(val)?;
        let mut vec = Vec::with_capacity(ary.len());
        for i in 0..ary.len() {
            let item: Value = ary.entry(i as isize)?;
            vec.push(ruby_to_json(item)?);
        }
        return Ok(serde_json::Value::Array(vec));
    }
    if val.is_kind_of(ruby.class_hash()) {
        let hash = RHash::try_convert(val)?;
        let mut map = serde_json::Map::new();
        hash.foreach(|k: Value, v: Value| {
            let ruby = unsafe { Ruby::get_unchecked() };
            let key: String = if k.is_kind_of(ruby.class_symbol()) {
                k.funcall("to_s", ())?
            } else {
                TryConvert::try_convert(k)?
            };
            map.insert(key, ruby_to_json(v)?);
            Ok(magnus::r_hash::ForEach::Continue)
        })?;
        return Ok(serde_json::Value::Object(map));
    }
    // fallback: .to_s
    let s: String = val.funcall("to_s", ())?;
    Ok(serde_json::Value::String(s))
}

// --------------------------------------------------------------------------
// Init
// --------------------------------------------------------------------------

pub fn init(_ruby: &magnus::Ruby, module: &magnus::RModule) -> Result<(), magnus::Error> {
    let ruby = unsafe { Ruby::get_unchecked() };
    let client_class = module.define_class("Client", ruby.class_object())?;
    client_class.define_singleton_method("new", function!(Client::rb_new, -1))?;
    client_class.define_method("get", method!(Client::get, -1))?;
    client_class.define_method("post", method!(Client::post, -1))?;
    client_class.define_method("put", method!(Client::put, -1))?;
    client_class.define_method("patch", method!(Client::patch, -1))?;
    client_class.define_method("delete", method!(Client::delete, -1))?;
    client_class.define_method("head", method!(Client::head, -1))?;
    client_class.define_method("options", method!(Client::options, -1))?;

    module.define_module_function("get", function!(wreq_get, -1))?;
    module.define_module_function("post", function!(wreq_post, -1))?;
    module.define_module_function("put", function!(wreq_put, -1))?;
    module.define_module_function("patch", function!(wreq_patch, -1))?;
    module.define_module_function("delete", function!(wreq_delete, -1))?;
    module.define_module_function("head", function!(wreq_head, -1))?;

    Ok(())
}
