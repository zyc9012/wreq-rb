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
end
