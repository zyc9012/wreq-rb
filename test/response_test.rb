# frozen_string_literal: true

require_relative "test_helper"

class ResponseTest < Minitest::Test
  def test_response_methods
    resp = Wreq.get("https://httpbin.org/get")
    assert_kind_of Integer, resp.status
    assert_kind_of String, resp.text
    assert_kind_of String, resp.url
    assert_kind_of Hash, resp.headers
    assert_includes resp.inspect, "Wreq::Response"
  end
end
