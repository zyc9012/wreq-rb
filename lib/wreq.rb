# frozen_string_literal: true

require_relative "wreq/version"
require_relative "wreq_rb/wreq_rb"

module Wreq
  # Convenience: create a default client and reuse it for module-level calls.
  #
  # All HTTP methods accept:
  #   url      - the request URL (required, first positional argument)
  #   options  - optional Hash with :headers, :body, :json, :form, :query,
  #              :timeout, :auth, :bearer, :basic, :proxy
  #
  # Examples:
  #
  #   # Simple GET
  #   resp = Wreq.get("https://httpbin.org/get")
  #   puts resp.status   # => 200
  #   puts resp.text     # => response body string
  #   puts resp.json     # => parsed Hash
  #
  #   # POST with JSON
  #   resp = Wreq.post("https://httpbin.org/post", json: { name: "wreq" })
  #
  #   # Custom headers
  #   resp = Wreq.get("https://httpbin.org/headers",
  #     headers: { "X-Custom" => "value" })
  #
  #   # Using a client for connection reuse
  #   client = Wreq::Client.new(
  #     cookies: true,
  #     redirect: 5,
  #     timeout: 30,
  #     proxy: "http://proxy:8080"
  #   )
  #   resp = client.get("https://example.com")
end
