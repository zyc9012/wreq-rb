use magnus::{
    method, prelude::*, Module, RHash, Ruby, Value,
};

use crate::error::generic_error;

/// Wraps a wreq::Response in a Ruby-accessible type.
#[magnus::wrap(class = "Wreq::Response", free_immediately)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    url: String,
    version: String,
    content_length: Option<u64>,
}

impl Response {
    pub fn new(
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        url: String,
        version: String,
        content_length: Option<u64>,
    ) -> Self {
        Self {
            status,
            headers,
            body,
            url,
            version,
            content_length,
        }
    }

    fn status(&self) -> u16 {
        self.status
    }

    fn text(&self) -> Result<String, magnus::Error> {
        String::from_utf8(self.body.clone()).map_err(|e| generic_error(e))
    }

    fn body_bytes(&self) -> Vec<u8> {
        self.body.clone()
    }

    fn headers(&self) -> Result<RHash, magnus::Error> {
        let ruby = unsafe { Ruby::get_unchecked() };
        let hash = ruby.hash_new();
        for (k, v) in &self.headers {
            hash.aset(k.as_str(), v.as_str())?;
        }
        Ok(hash)
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn http_version(&self) -> String {
        self.version.clone()
    }

    fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }

    fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }

    fn json(&self) -> Result<Value, magnus::Error> {
        let ruby = unsafe { Ruby::get_unchecked() };
        let text = self.text()?;
        let val: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| generic_error(e))?;
        json_to_ruby(&ruby, &val)
    }

    fn inspect(&self) -> String {
        format!(
            "#<Wreq::Response status={} url={:?}>",
            self.status, self.url
        )
    }

    fn to_s(&self) -> Result<String, magnus::Error> {
        self.text()
    }
}

pub fn json_to_ruby(ruby: &Ruby, val: &serde_json::Value) -> Result<Value, magnus::Error> {
    match val {
        serde_json::Value::Null => Ok(ruby.qnil().as_value()),
        serde_json::Value::Bool(b) => {
            if *b {
                Ok(ruby.qtrue().as_value())
            } else {
                Ok(ruby.qfalse().as_value())
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(ruby.integer_from_i64(i).as_value())
            } else if let Some(f) = n.as_f64() {
                Ok(ruby.float_from_f64(f).as_value())
            } else {
                Ok(ruby.qnil().as_value())
            }
        }
        serde_json::Value::String(s) => Ok(ruby.str_new(s).as_value()),
        serde_json::Value::Array(arr) => {
            let ary = ruby.ary_new_capa(arr.len());
            for item in arr {
                ary.push(json_to_ruby(ruby, item)?)?;
            }
            Ok(ary.as_value())
        }
        serde_json::Value::Object(map) => {
            let hash = ruby.hash_new();
            for (k, v) in map {
                hash.aset(ruby.str_new(k), json_to_ruby(ruby, v)?)?;
            }
            Ok(hash.as_value())
        }
    }
}

pub fn init(ruby: &magnus::Ruby, module: &magnus::RModule) -> Result<(), magnus::Error> {
    let class = module.define_class("Response", ruby.class_object())?;
    class.define_method("status", method!(Response::status, 0))?;
    class.define_method("code", method!(Response::status, 0))?;
    class.define_method("text", method!(Response::text, 0))?;
    class.define_method("body", method!(Response::text, 0))?;
    class.define_method("body_bytes", method!(Response::body_bytes, 0))?;
    class.define_method("headers", method!(Response::headers, 0))?;
    class.define_method("url", method!(Response::url, 0))?;
    class.define_method("version", method!(Response::http_version, 0))?;
    class.define_method("content_length", method!(Response::content_length, 0))?;
    class.define_method("success?", method!(Response::is_success, 0))?;
    class.define_method("redirect?", method!(Response::is_redirect, 0))?;
    class.define_method("client_error?", method!(Response::is_client_error, 0))?;
    class.define_method("server_error?", method!(Response::is_server_error, 0))?;
    class.define_method("json", method!(Response::json, 0))?;
    class.define_method("inspect", method!(Response::inspect, 0))?;
    class.define_method("to_s", method!(Response::to_s, 0))?;
    Ok(())
}
