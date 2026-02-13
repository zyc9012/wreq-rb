# frozen_string_literal: true

require_relative "test_helper"

class ClientTest < Minitest::Test
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

  def test_redirect_client
    client = Wreq::Client.new(redirect: 5)
    resp = client.get("https://httpbin.org/redirect/2")
    assert_equal 200, resp.status
  end
end
