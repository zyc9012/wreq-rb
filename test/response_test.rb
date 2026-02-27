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

  def test_transfer_size_with_compressed_response
    # /gzip returns gzip-compressed data; transfer_size should be smaller than body
    resp = Wreq.get("https://httpbin.org/gzip")
    assert_equal 200, resp.status

    body_size = resp.body_bytes.length
    transfer = resp.transfer_size

    assert_kind_of Integer, transfer
    assert transfer > 0, "transfer_size should be positive"
    assert transfer < body_size,
      "transfer_size (#{transfer}) should be less than decompressed body (#{body_size}) for gzip response"
  end

  def test_transfer_size_with_uncompressed_response
    # /robots.txt is small and typically not compressed; sizes should match
    resp = Wreq.get("https://httpbin.org/robots.txt")
    assert_equal 200, resp.status

    body_size = resp.body_bytes.length
    transfer = resp.transfer_size

    assert_kind_of Integer, transfer
    assert_equal body_size, transfer,
      "transfer_size (#{transfer}) should equal body size (#{body_size}) for uncompressed response"
  end
end
