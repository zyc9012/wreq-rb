# frozen_string_literal: true

require_relative "test_helper"

class EmulationTest < Minitest::Test
  def test_default_emulation_uses_chrome_ua
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
    client = Wreq::Client.new(emulation: "chrome_143", user_agent: "my-bot/2.0")
    resp = client.get("https://httpbin.org/user-agent")
    body = resp.json
    assert_equal "my-bot/2.0", body["user-agent"]
  end
end
