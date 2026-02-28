# wreq-rb

Ruby bindings for the [wreq](https://github.com/0x676e67/wreq) Rust HTTP client — featuring TLS fingerprint emulation, HTTP/2 support, cookie handling, and proxy support.

## Installation

Add to your Gemfile:

```ruby
gem "wreq-rb"
```

Then run:

```bash
bundle install
```

> **Build prerequisites:** You need Rust (1.85+), Clang, CMake, and Perl installed, since wreq compiles BoringSSL from source. See [wreq's build guide](https://github.com/0x676e67/wreq#building) for details.

## Quick Start

```ruby
require "wreq-rb"

# Simple GET request
resp = Wreq.get("https://httpbin.org/get")
puts resp.status   # => 200
puts resp.text     # => response body as string
puts resp.json     # => parsed Ruby Hash

# POST with JSON body
resp = Wreq.post("https://httpbin.org/post", json: { name: "wreq", version: 1 })

# POST with form data
resp = Wreq.post("https://httpbin.org/post", form: { key: "value" })

# Custom headers
resp = Wreq.get("https://httpbin.org/headers",
  headers: { "X-Custom" => "value", "Accept" => "application/json" })

# Query parameters
resp = Wreq.get("https://httpbin.org/get", query: { foo: "bar", page: "1" })

# Authentication
resp = Wreq.get("https://httpbin.org/bearer", bearer: "my-token")
resp = Wreq.get("https://httpbin.org/basic-auth/user/pass", basic: ["user", "pass"])

# Browser emulation (enabled by default)
resp = Wreq.get("https://tls.peet.ws/api/all", emulation: "chrome_143")
```

## Using a Client

For best performance, create a `Wreq::Client` and reuse it across requests (connections are pooled internally):

```ruby
client = Wreq::Client.new(
  user_agent: "MyApp/1.0",
  timeout: 30,                 # total timeout in seconds
  connect_timeout: 5,          # connection timeout
  read_timeout: 15,            # read timeout
  redirect: 10,                # follow up to 10 redirects (false to disable)
  cookie_store: true,          # enable cookie jar
  proxy: "http://proxy:8080",  # proxy URL (supports http, https, socks5)
  proxy_user: "user",          # proxy auth
  proxy_pass: "pass",
  no_proxy: true,              # disable all proxies (including env-vars)
  https_only: false,           # restrict to HTTPS
  verify_host: true,           # verify TLS hostname (default: true)
  verify_cert: true,           # verify TLS certificate (default: true)
  http1_only: false,           # force HTTP/1.1 only
  http2_only: false,           # force HTTP/2 only
  gzip: true,                  # enable gzip decompression
  brotli: true,                # enable brotli decompression
  deflate: true,               # enable deflate decompression
  zstd: true,                  # enable zstd decompression
  emulation: "chrome_143",     # browser emulation (enabled by default)
  headers: {                   # default headers for all requests
    "Accept" => "application/json"
  }
)

resp = client.get("https://api.example.com/data")
resp = client.post("https://api.example.com/data", json: { key: "value" })
```

## HTTP Methods

All methods are available on both `Wreq` (module-level) and `Wreq::Client` (instance-level):

| Method | Usage |
|--------|-------|
| `get(url, **opts)` | GET request |
| `post(url, **opts)` | POST request |
| `put(url, **opts)` | PUT request |
| `patch(url, **opts)` | PATCH request |
| `delete(url, **opts)` | DELETE request |
| `head(url, **opts)` | HEAD request |
| `options(url, **opts)` | OPTIONS request |

### Per-Request Options

Pass an options hash as the second argument to any HTTP method:

| Option | Type | Description |
|--------|------|-------------|
| `headers` | Hash | Request headers |
| `body` | String | Raw request body |
| `json` | Hash/Array | JSON-serialized body (sets Content-Type) |
| `form` | Hash | URL-encoded form body |
| `query` | Hash | URL query parameters |
| `timeout` | Float | Per-request timeout (seconds) |
| `auth` | String | Raw Authorization header |
| `bearer` | String | Bearer token |
| `basic` | Array | `[username, password]` for Basic auth |
| `proxy` | String | Per-request proxy URL |
| `emulation` | String/Boolean | Per-request emulation override |

## Browser Emulation

wreq-rb emulates real browser TLS fingerprints, HTTP/2 settings, and headers by default. **Chrome 143 is used when no emulation is specified.**

```ruby
# Default: Chrome 143 emulation (automatic)
resp = Wreq.get("https://tls.peet.ws/api/all")

# Explicit browser emulation
client = Wreq::Client.new(emulation: "firefox_146")
client = Wreq::Client.new(emulation: "safari_18_5")
client = Wreq::Client.new(emulation: "edge_142")

# Disable emulation entirely
client = Wreq::Client.new(emulation: false)

# Emulation + custom user-agent (user_agent overrides emulation's UA)
client = Wreq::Client.new(emulation: "chrome_143", user_agent: "MyBot/1.0")

# Per-request emulation override
resp = client.get("https://example.com", emulation: "safari_26_2")
```

### Supported Browsers

| Browser | Example values |
|---------|---------------|
| Chrome | `chrome_100` .. `chrome_143` |
| Firefox | `firefox_109`, `firefox_146`, `firefox_private_135` |
| Safari | `safari_15.3` .. `safari_26.2`, `safari_ios_26`, `safari_ipad_18` |
| Edge | `edge_101` .. `edge_142` |
| Opera | `opera_116` .. `opera_119` |
| OkHttp | `okhttp_3_9` .. `okhttp_5` |

## Response

The `Wreq::Response` object provides:

| Method | Returns | Description |
|--------|---------|-------------|
| `status` / `code` | Integer | HTTP status code |
| `text` / `body` | String | Response body as string |
| `body_bytes` | Array | Raw bytes |
| `headers` | Hash | Response headers |
| `json` | Hash/Array | JSON-parsed body |
| `url` | String | Final URL (after redirects) |
| `version` | String | HTTP version |
| `content_length` | Integer/nil | Content length if known |
| `transfer_size` | Integer/nil | Bytes transferred over the wire |
| `success?` | Boolean | Status 2xx? |
| `redirect?` | Boolean | Status 3xx? |
| `client_error?` | Boolean | Status 4xx? |
| `server_error?` | Boolean | Status 5xx? |

## Building from Source

```bash
bundle install
bundle exec rake compile
bundle exec rake test
```
