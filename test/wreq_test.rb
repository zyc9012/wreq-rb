# frozen_string_literal: true

require "minitest/autorun"
require "wreq"

class WreqClientTest < Minitest::Test
  def test_simple_get
    resp = Wreq.get("https://httpbin.org/get")
    assert_equal 200, resp.status
    assert resp.success?
    refute resp.text.empty?
  end

  def test_get_with_headers
    resp = Wreq.get("https://httpbin.org/headers",
      headers: { "X-Test-Header" => "hello" })
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "hello", body["headers"]["X-Test-Header"]
  end

  def test_post_json
    resp = Wreq.post("https://httpbin.org/post",
      json: { "name" => "wreq", "version" => 1 })
    assert_equal 200, resp.status
    body = resp.json
    data = body["json"] || JSON.parse(body["data"])
    assert_equal "wreq", data["name"]
  end

  def test_post_form
    resp = Wreq.post("https://httpbin.org/post",
      form: { "key" => "value" })
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "value", body["form"]["key"]
  end

  def test_query_params
    resp = Wreq.get("https://httpbin.org/get",
      query: { "foo" => "bar", "baz" => "qux" })
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "bar", body["args"]["foo"]
    assert_equal "qux", body["args"]["baz"]
  end

  def test_client_with_options
    client = Wreq::Client.new(
      user_agent: "wreq-rb-test/0.1",
      timeout: 30
    )
    resp = client.get("https://httpbin.org/user-agent")
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "wreq-rb-test/0.1", body["user-agent"]
  end

  def test_response_methods
    resp = Wreq.get("https://httpbin.org/get")
    assert_kind_of Integer, resp.status
    assert_kind_of String, resp.text
    assert_kind_of String, resp.url
    assert_kind_of Hash, resp.headers
    assert_includes resp.inspect, "Wreq::Response"
  end

  def test_head_request
    resp = Wreq.head("https://httpbin.org/get")
    assert_equal 200, resp.status
  end

  def test_put_request
    resp = Wreq.put("https://httpbin.org/put",
      body: "test body")
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "test body", body["data"]
  end

  def test_delete_request
    resp = Wreq.delete("https://httpbin.org/delete")
    assert_equal 200, resp.status
  end

  def test_patch_request
    resp = Wreq.patch("https://httpbin.org/patch",
      json: { "patched" => true })
    assert_equal 200, resp.status
  end

  def test_bearer_auth
    resp = Wreq.get("https://httpbin.org/bearer",
      bearer: "test-token-123")
    assert_equal 200, resp.status
  end

  def test_basic_auth
    resp = Wreq.get("https://httpbin.org/basic-auth/user/pass",
      basic: ["user", "pass"])
    assert_equal 200, resp.status
  end

  def test_redirect_client
    client = Wreq::Client.new(redirect: 5)
    resp = client.get("https://httpbin.org/redirect/2")
    assert_equal 200, resp.status
  end

  # ---- Emulation tests ----

  def test_default_emulation_uses_chrome_ua
    # Default client should have Chrome user-agent from emulation
    resp = Wreq.get("https://httpbin.org/user-agent")
    body = resp.json
    assert_match(/Chrome/, body["user-agent"])
  end

  def test_explicit_emulation_firefox
    client = Wreq::Client.new(emulation: "firefox_146")
    resp = client.get("https://httpbin.org/user-agent")
    body = resp.json
    assert_match(/Firefox/, body["user-agent"])
  end

  def test_emulation_disabled
    client = Wreq::Client.new(emulation: false, user_agent: "custom-agent/1.0")
    resp = client.get("https://httpbin.org/user-agent")
    body = resp.json
    assert_equal "custom-agent/1.0", body["user-agent"]
  end

  def test_emulation_with_user_agent_override
    # User-agent should override emulation's default UA
    client = Wreq::Client.new(emulation: "chrome_143", user_agent: "my-bot/2.0")
    resp = client.get("https://httpbin.org/user-agent")
    body = resp.json
    assert_equal "my-bot/2.0", body["user-agent"]
  end

  # ---- GVL release tests ----

  def test_gvl_released_during_network_io
    # If the GVL is properly released, multiple threads can make
    # requests concurrently and finish faster than sequentially.
    client = Wreq::Client.new(timeout: 10)
    num_threads = 4

    # httpbin.org/delay/N sleeps for N seconds server-side
    delay = 2

    start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    threads = num_threads.times.map do
      Thread.new do
        resp = client.get("https://httpbin.org/delay/#{delay}")
        resp.status
      end
    end
    results = threads.map(&:value)
    elapsed = Process.clock_gettime(Process::CLOCK_MONOTONIC) - start

    # All requests should succeed
    results.each { |status| assert_equal 200, status }

    # If GVL were held, this would take ~num_threads * delay seconds.
    # With GVL released, requests run in parallel: ~delay seconds + overhead.
    max_expected = delay * num_threads - 1  # well under sequential time
    assert elapsed < max_expected,
      "Expected concurrent requests to finish in <#{max_expected}s, " \
      "but took #{elapsed.round(2)}s (GVL may not be released)"
  end

  def test_threads_can_run_ruby_during_request
    # Verify that a Ruby thread can do work while another is blocked on I/O.
    client = Wreq::Client.new(timeout: 10)
    ruby_thread_ran = false

    request_thread = Thread.new do
      client.get("https://httpbin.org/delay/2")
    end

    # Give the request thread a moment to start I/O
    sleep 0.1

    # This Ruby thread should be able to run while the request is in flight
    ruby_thread = Thread.new do
      ruby_thread_ran = true
    end
    ruby_thread.join(3)

    request_thread.join

    assert ruby_thread_ran,
      "Ruby thread could not run while request was in flight (GVL not released)"
  end
end
