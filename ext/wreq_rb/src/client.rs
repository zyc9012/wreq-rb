use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use magnus::{
    function, method, prelude::*, Module, RArray, RHash, Ruby,
    try_convert::TryConvert, Value,
};
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use std::net::IpAddr;
use wreq::header::{HeaderMap, HeaderName, HeaderValue, OrigHeaderMap};
use wreq::tls::TlsVersion;
use wreq::IntoEmulation;
use wreq_util::{Emulation as BrowserEmulation, Platform as EmulationPlatform, Profile as BrowserProfile};

use crate::error::{generic_error, to_magnus_error, wreq_error};
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
    transfer_size: Option<u64>,
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
    let transfer_size_handle = resp.transfer_size_handle().cloned();
    let body = resp.bytes().await?.to_vec();
    let transfer_size = transfer_size_handle.map(|h| h.get());
    Ok(ResponseData { status, headers, body, url, version, content_length, transfer_size })
}

// --------------------------------------------------------------------------
// Batch execution
// --------------------------------------------------------------------------

const DEFAULT_BATCH_CONCURRENCY: usize = 16;

/// Per-item batch result as pure Rust types (no Ruby objects). Errors are kept
/// as messages so one failure never discards the rest of the batch.
enum BatchItem {
    Ok(ResponseData),
    Err(String),
}

/// Outcome of a whole batch performed outside the GVL.
enum BatchOutcome {
    Done(Vec<BatchItem>),
    Interrupted,
}

/// Run every request concurrently with at most `concurrency` in flight,
/// returning results in input order.
async fn execute_batch(reqs: Vec<wreq::RequestBuilder>, concurrency: usize) -> Vec<BatchItem> {
    let permits = Arc::new(Semaphore::new(concurrency));
    let mut set: JoinSet<(usize, BatchItem)> = JoinSet::new();

    for (idx, req) in reqs.into_iter().enumerate() {
        let permits = Arc::clone(&permits);
        set.spawn(async move {
            let _permit = match permits.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return (idx, BatchItem::Err("batch semaphore closed".to_owned())),
            };
            let item = match execute_request(req).await {
                Ok(data) => BatchItem::Ok(data),
                Err(e) => BatchItem::Err(e.to_string()),
            };
            (idx, item)
        });
    }

    let mut slots: Vec<Option<BatchItem>> = Vec::new();
    slots.resize_with(set.len(), || None);
    while let Some(joined) = set.join_next().await {
        // A JoinError means the task panicked or was aborted; its slot is left
        // empty and filled with a generic error below.
        if let Ok((idx, item)) = joined {
            slots[idx] = Some(item);
        }
    }

    slots
        .into_iter()
        .map(|slot| slot.unwrap_or_else(|| BatchItem::Err("request task failed".to_owned())))
        .collect()
}

// --------------------------------------------------------------------------
// Emulation helpers
// --------------------------------------------------------------------------

/// The default browser profile to apply when none is specified.
const DEFAULT_EMULATION: BrowserProfile = BrowserProfile::Chrome149;

/// Parse a Ruby string like "chrome_143" into a BrowserProfile variant.
fn parse_emulation(name: &str) -> Result<BrowserProfile, magnus::Error> {
    let json_val = serde_json::Value::String(name.to_string());
    serde_json::from_value::<BrowserProfile>(json_val)
        .map_err(|_| generic_error(format!("unknown emulation: '{}'. Use names like 'chrome_148', 'firefox_151', 'safari_18.5', etc.", name)))
}

/// Parse a Ruby string like "windows" into an EmulationPlatform variant.
fn parse_emulation_os(name: &str) -> Result<EmulationPlatform, magnus::Error> {
    let json_val = serde_json::Value::String(name.to_string());
    serde_json::from_value::<EmulationPlatform>(json_val)
        .map_err(|_| generic_error("unknown emulation_os. Use: 'windows', 'macos', 'linux', 'android', 'ios'"))
}

/// Build a BrowserEmulation from a profile and an optional platform from the opts hash.
fn build_emulation_option(
    profile: BrowserProfile,
    opts: &RHash,
) -> Result<BrowserEmulation, magnus::Error> {
    let platform = match hash_get_string(opts, "emulation_os")? {
        Some(platform_name) => parse_emulation_os(&platform_name)?,
        None => EmulationPlatform::default(),
    };
    Ok(BrowserEmulation::builder()
        .profile(profile)
        .platform(platform)
        .build())
}

fn apply_request_emulation(
    req: wreq::RequestBuilder,
    option: BrowserEmulation,
    header_order: Option<&OrigHeaderMap>,
) -> wreq::RequestBuilder {
    let mut emulation = option.into_emulation();
    if let Some(header_order) = header_order {
        let mut merged_order = header_order.clone();
        merged_order.extend(emulation.orig_headers);
        emulation.orig_headers = merged_order;
    }
    req.emulation(emulation)
}

// --------------------------------------------------------------------------
// Ruby Client
// --------------------------------------------------------------------------

#[magnus::wrap(class = "Wreq::Client", free_immediately)]
struct Client {
    inner: wreq::Client,
    header_order: Option<OrigHeaderMap>,
    cancel_token: std::sync::Mutex<CancellationToken>,
}

impl Client {
    /// Wreq::Client.new or Wreq::Client.new(options_hash)
    fn rb_new(args: &[Value]) -> Result<Self, magnus::Error> {
        let opts: Option<RHash> = if args.is_empty() {
            None
        } else {
            Some(RHash::try_convert(args[0])?)
        };

        let mut builder = wreq::Client::builder()
            .retry(wreq::retry::Policy::never());
        let mut header_order = None;

        if let Some(opts) = opts {
            // Apply header_order BEFORE emulation so the user's ordering takes precedence
            if let Some(ary) = hash_get_array(&opts, "header_order")? {
                let orig = array_to_orig_header_map(ary)?;
                builder = builder.orig_headers(orig.clone());
                header_order = Some(orig);
            }

            if let Some(val) = hash_get_value(&opts, "emulation")? {
                let ruby = unsafe { Ruby::get_unchecked() };
                if val.is_kind_of(ruby.class_false_class()) {
                    // emulation: false — skip emulation entirely
                } else if val.is_kind_of(ruby.class_true_class()) {
                    let opt = build_emulation_option(DEFAULT_EMULATION, &opts)?;
                    builder = builder.emulation(opt);
                } else {
                    let name: String = TryConvert::try_convert(val)?;
                    let emu = parse_emulation(&name)?;
                    let opt = build_emulation_option(emu, &opts)?;
                    builder = builder.emulation(opt);
                }
            } else {
                let opt = build_emulation_option(DEFAULT_EMULATION, &opts)?;
                builder = builder.emulation(opt);
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

            if let Some(enabled) = hash_get_bool(&opts, "cookie_store")? {
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

            if let Some(v) = hash_get_bool(&opts, "verify_host")? {
                builder = builder.tls_verify_hostname(v);
            }

            if let Some(v) = hash_get_bool(&opts, "verify_cert")? {
                builder = builder.tls_cert_verification(v);
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

            if let Some(v) = hash_get_bool(&opts, "referer")? {
                builder = builder.referer(v);
            }

            if let Some(n) = hash_get_usize(&opts, "pool_max_idle_per_host")? {
                builder = builder.pool_max_idle_per_host(n);
            }

            if let Some(n) = hash_get_usize(&opts, "pool_max_size")? {
                builder = builder.pool_max_size(n);
            }

            if let Some(v) = hash_get_bool(&opts, "tcp_nodelay")? {
                builder = builder.tcp_nodelay(v);
            }

            if let Some(t) = hash_get_float(&opts, "tcp_keepalive")? {
                builder = builder.tcp_keepalive(Duration::from_secs_f64(t));
            }

            if let Some(addr_str) = hash_get_string(&opts, "local_address")? {
                let addr: IpAddr = addr_str.parse()
                    .map_err(|_| generic_error(format!("invalid IP address: '{}'", addr_str)))?;
                builder = builder.local_address(addr);
            }

            if let Some(v) = hash_get_bool(&opts, "tls_sni")? {
                builder = builder.tls_sni(v);
            }

            if let Some(s) = hash_get_string(&opts, "min_tls_version")? {
                builder = builder.tls_min_version(parse_tls_version(&s)?);
            }

            if let Some(s) = hash_get_string(&opts, "max_tls_version")? {
                builder = builder.tls_max_version(parse_tls_version(&s)?);
            }
        } else {
            builder = builder.emulation(DEFAULT_EMULATION);
        }

        let client = builder.build().map_err(to_magnus_error)?;
        Ok(Client {
            inner: client,
            header_order,
            cancel_token: std::sync::Mutex::new(CancellationToken::new()),
        })
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

    fn cancel(&self) {
        // Replace the cancel token first so new requests use a fresh token,
        // then cancel the old one to unblock all current in-flight select!s.
        let old_token = {
            let mut guard = self.cancel_token.lock().unwrap_or_else(|e| e.into_inner());
            let old = guard.clone();
            *guard = CancellationToken::new();
            old
        };
        old_token.cancel();
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
            req = apply_request_options(req, &opts, self.header_order.as_ref())?;
        }

        let client_token = self.cancel_token.lock().unwrap_or_else(|e| e.into_inner()).clone();

        // Release the GVL so other Ruby threads can run during I/O.
        let outcome: RequestOutcome = unsafe {
            without_gvl(|thread_token| {
                runtime().block_on(async {
                    tokio::select! {
                        biased;
                        _ = thread_token.cancelled() => RequestOutcome::Interrupted,
                        _ = client_token.cancelled() => RequestOutcome::Interrupted,
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
        Ok(Response::new(data.status, data.headers, data.body, data.url, data.version, data.content_length, data.transfer_size))
    }

    /// Wreq::Client#request_batch(specs) or #request_batch(specs, options)
    fn request_batch(&self, args: &[Value]) -> Result<RArray, magnus::Error> {
        if args.is_empty() {
            return Err(generic_error("an array of requests is required"));
        }
        let specs = RArray::try_convert(args[0])?;

        let opts: Option<RHash> = if args.len() > 1 {
            Some(RHash::try_convert(args[1])?)
        } else {
            None
        };

        let concurrency = match opts.as_ref() {
            Some(o) => hash_get_usize(o, "concurrency")?.unwrap_or(DEFAULT_BATCH_CONCURRENCY),
            None => DEFAULT_BATCH_CONCURRENCY,
        };
        if concurrency == 0 {
            return Err(generic_error("concurrency must be >= 1"));
        }

        // All Ruby -> Rust conversion happens here, while we still hold the GVL.
        let mut reqs: Vec<wreq::RequestBuilder> = Vec::with_capacity(specs.len());
        for spec in specs.into_iter() {
            reqs.push(self.build_request(spec, opts.as_ref())?);
        }

        let ruby = unsafe { Ruby::get_unchecked() };
        if reqs.is_empty() {
            return Ok(ruby.ary_new());
        }

        let client_token = self.cancel_token.lock().unwrap_or_else(|e| e.into_inner()).clone();

        // One GVL release covering the whole batch.
        let outcome: BatchOutcome = unsafe {
            without_gvl(|thread_token| {
                runtime().block_on(async {
                    tokio::select! {
                        biased;
                        _ = thread_token.cancelled() => BatchOutcome::Interrupted,
                        _ = client_token.cancelled() => BatchOutcome::Interrupted,
                        items = execute_batch(reqs, concurrency) => BatchOutcome::Done(items),
                    }
                })
            })
        };

        let items = match outcome {
            BatchOutcome::Done(items) => items,
            BatchOutcome::Interrupted => return Err(generic_error("request interrupted")),
        };

        // Back under the GVL: Ruby objects may be created again.
        let results = ruby.ary_new_capa(items.len());
        for item in items {
            match item {
                BatchItem::Ok(d) => {
                    let resp = Response::new(
                        d.status, d.headers, d.body, d.url, d.version, d.content_length,
                        d.transfer_size,
                    );
                    results.push(ruby.obj_wrap(resp))?;
                }
                BatchItem::Err(msg) => {
                    let err: Value = wreq_error().funcall("new", (msg,))?;
                    results.push(err)?;
                }
            }
        }
        Ok(results)
    }

    /// Convert a single batch spec into a RequestBuilder. Accepted forms:
    ///   "https://example.com"      -> GET
    ///   { method:, url:, **opts }
    fn build_request(
        &self,
        spec: Value,
        shared: Option<&RHash>,
    ) -> Result<wreq::RequestBuilder, magnus::Error> {
        let mut method = wreq::Method::GET;
        let url: String;
        let mut item_opts: Option<RHash> = None;

        if let Some(hash) = RHash::from_value(spec) {
            url = hash_get_string(&hash, "url")?
                .ok_or_else(|| generic_error("each request hash requires a :url"))?;
            if let Some(val) = hash_get_value(&hash, "method")? {
                method = value_to_method(val)?;
            }
            item_opts = Some(hash);
        } else {
            url = TryConvert::try_convert(spec)?;
        }

        let mut req = self.inner.request(method, &url);
        if let Some(shared) = shared {
            req = apply_request_options(req, shared, self.header_order.as_ref())?;
        }
        if let Some(item_opts) = item_opts {
            req = apply_request_options(req, &item_opts, self.header_order.as_ref())?;
        }
        Ok(req)
    }
}

/// Parse a String or Symbol like "post" / :post into an HTTP method.
fn value_to_method(val: Value) -> Result<wreq::Method, magnus::Error> {
    let name: String = val.funcall("to_s", ())?;
    name.to_uppercase()
        .parse()
        .map_err(|_| generic_error(format!("invalid HTTP method: {}", name)))
}

fn apply_request_options(
    mut req: wreq::RequestBuilder,
    opts: &RHash,
    header_order: Option<&OrigHeaderMap>,
) -> Result<wreq::RequestBuilder, magnus::Error> {
    if let Some(val) = hash_get_value(opts, "emulation")? {
        let ruby = unsafe { Ruby::get_unchecked() };
        if val.is_kind_of(ruby.class_false_class()) {
            // emulation: false — no per-request emulation override
        } else if val.is_kind_of(ruby.class_true_class()) {
            let opt = build_emulation_option(DEFAULT_EMULATION, opts)?;
            req = apply_request_emulation(req, opt, header_order);
        } else {
            let name: String = TryConvert::try_convert(val)?;
            let emu = parse_emulation(&name)?;
            let opt = build_emulation_option(emu, opts)?;
            req = apply_request_emulation(req, opt, header_order);
        }
    }

    if let Some(hdr_hash) = hash_get_hash(opts, "headers")? {
        let hmap = hash_to_header_map(&hdr_hash)?;
        req = req.headers(hmap);
    }

    if let Some(body_str) = hash_get_string(opts, "body")? {
        req = req.body(body_str);
    }

    if let Some(json_val) = hash_get_value(opts, "json")? {
        let ruby = unsafe { Ruby::get_unchecked() };
        let json_module: Value = ruby.class_object().const_get("JSON")?;
        let json_str: String = json_module.funcall("generate", (json_val,))?;
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

fn hash_get_usize(hash: &RHash, key: &str) -> Result<Option<usize>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(TryConvert::try_convert(v)?)),
        None => Ok(None),
    }
}

fn parse_tls_version(s: &str) -> Result<TlsVersion, magnus::Error> {
    match s {
        "tls1.0" | "tls_1_0" | "1.0" => Ok(TlsVersion::TLS_1_0),
        "tls1.1" | "tls_1_1" | "1.1" => Ok(TlsVersion::TLS_1_1),
        "tls1.2" | "tls_1_2" | "1.2" => Ok(TlsVersion::TLS_1_2),
        "tls1.3" | "tls_1_3" | "1.3" => Ok(TlsVersion::TLS_1_3),
        _ => Err(generic_error(format!(
            "unknown TLS version '{}'. Use: 'tls1.2', 'tls1.3'", s
        ))),
    }
}

fn hash_get_hash(hash: &RHash, key: &str) -> Result<Option<RHash>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(RHash::try_convert(v)?)),
        None => Ok(None),
    }
}

fn hash_get_array(hash: &RHash, key: &str) -> Result<Option<RArray>, magnus::Error> {
    match hash_get_value(hash, key)? {
        Some(v) => Ok(Some(RArray::try_convert(v)?)),
        None => Ok(None),
    }
}

fn array_to_orig_header_map(ary: RArray) -> Result<OrigHeaderMap, magnus::Error> {
    let mut orig = OrigHeaderMap::with_capacity(ary.len());
    for elem in ary.into_iter() {
        let ruby = unsafe { Ruby::get_unchecked() };
        let name_str: String = if elem.is_kind_of(ruby.class_symbol()) {
            elem.funcall("to_s", ())?  
        } else {
            TryConvert::try_convert(elem)?
        };
        orig.insert(name_str);
    }
    Ok(orig)
}

fn hash_to_header_map(hash: &RHash) -> Result<HeaderMap, magnus::Error> {
    let mut hmap = HeaderMap::new();
    hash.foreach(|k: Value, v: Value| {
        if v.is_nil() {
            return Ok(magnus::r_hash::ForEach::Continue);
        }
        let ruby = unsafe { Ruby::get_unchecked() };
        let ks: String = if k.is_kind_of(ruby.class_symbol()) {
            k.funcall("to_s", ())?
        } else {
            TryConvert::try_convert(k)?
        };
        let vs: String = v.funcall("to_s", ())?;
        let name =
            HeaderName::from_bytes(ks.as_bytes()).map_err(|e| generic_error(e))?;
        let value = HeaderValue::from_str(&vs).map_err(|e| generic_error(e))?;
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
            k.funcall("to_s", ())?
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
    client_class.define_method("request_batch", method!(Client::request_batch, -1))?;
    client_class.define_method("cancel", method!(Client::cancel, 0))?;

    module.define_module_function("get", function!(wreq_get, -1))?;
    module.define_module_function("post", function!(wreq_post, -1))?;
    module.define_module_function("put", function!(wreq_put, -1))?;
    module.define_module_function("patch", function!(wreq_patch, -1))?;
    module.define_module_function("delete", function!(wreq_delete, -1))?;
    module.define_module_function("head", function!(wreq_head, -1))?;

    Ok(())
}
