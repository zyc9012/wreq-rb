# frozen_string_literal: true

require_relative "test_helper"

class ClientTest < Minitest::Test
  def test_client_with_options
    client = Wreq::Client.new(
      user_agent: "wreq-rb-test/0.1",
      timeout: 30
    )
    resp = client.get("https://httpbun.com/headers")
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "wreq-rb-test/0.1", body["headers"]["User-Agent"]
  end

  def test_redirect_client
    client = Wreq::Client.new(redirect: 5)
    resp = client.get("https://httpbun.com/redirect/2")
    assert_equal 200, resp.status
  end

  def test_http1_only
    client = Wreq::Client.new(http1_only: true)
    resp = client.get("https://httpbun.com/get")
    assert_equal 200, resp.status
    assert_equal "HTTP/1.1", resp.version,
      "Expected HTTP/1.1 when http1_only: true, got #{resp.version}"
  end

  def test_http2_only
    client = Wreq::Client.new(http2_only: true)
    resp = client.get("https://httpbun.com/get")
    assert_equal 200, resp.status
    assert_equal "HTTP/2.0", resp.version,
      "Expected HTTP/2.0 when http2_only: true, got #{resp.version}"
  end

  def test_default_headers_with_mixed_types
    client = Wreq::Client.new(
      headers: { :"X-Symbol" => 99, "X-Nil" => nil, "X-Str" => "ok" }
    )
    resp = client.get("https://httpbun.com/headers")
    assert_equal 200, resp.status
    body = resp.json
    assert_equal "99", body["headers"]["X-Symbol"]
    assert_nil body["headers"]["X-Nil"]
    assert_equal "ok", body["headers"]["X-Str"]
  end

  def test_header_order
    order = ["x-zzz", "x-aaa", "x-ccc"]
    client = Wreq::Client.new(
      emulation: false,
      http1_only: true,
      no_proxy: true,
      header_order: order,
      headers: { "x-aaa" => "1", "x-ccc" => "2", "x-zzz" => "3" }
    )
    received = capture_wire_headers { |url| client.get(url) }

    positions = order.map { |h| received.index(h) }.compact
    assert_equal order.size, positions.size,
      "Not all target headers found in: #{received.inspect}"
    assert_equal positions.sort, positions,
      "Expected #{order.inspect} in order, got positions #{positions.inspect} in: #{received.inspect}"
  end

  def test_header_order_takes_precedence_over_emulation
    # Chrome emulation normally puts "user-agent" before "accept". We reverse
    # that relationship to prove the user's header_order wins.
    order = ["host", "accept", "user-agent"]
    client = Wreq::Client.new(
      emulation: "chrome_145",
      http1_only: true,
      no_proxy: true,
      header_order: order
    )
    received = capture_wire_headers { |url| client.get(url) }

    positions = order.map { |h| received.index(h) }.compact
    assert_equal order.size, positions.size,
      "Not all target headers found in: #{received.inspect}"
    assert_equal positions.sort, positions,
      "Expected user's header_order #{order.inspect} to take precedence; " \
      "got positions #{positions.inspect} in: #{received.inspect}"
  end

  def test_header_order_takes_precedence_over_request_emulation
    order = ["host", "accept", "user-agent"]
    custom_user_agent = "wreq-rb-request-test/0.1"
    client = Wreq::Client.new(
      emulation: false,
      http1_only: true,
      no_proxy: true,
      header_order: order
    )
    received = capture_wire_headers(include_values: true) do |url|
      client.get(url,
        emulation: "chrome_145",
        headers: { "User-Agent" => custom_user_agent })
    end
    header_names = received.map { |line| line.split(":", 2).first.downcase }

    positions = order.map { |header| header_names.index(header) }.compact
    assert_equal order.size, positions.size,
      "Not all target headers found in: #{received.inspect}"
    assert_equal positions.sort, positions,
      "Expected client's header_order #{order.inspect} to take precedence " \
      "over request emulation; got positions #{positions.inspect} in: #{received.inspect}"
    user_agent = received.find { |line| line.downcase.start_with?("user-agent:") }
    assert_equal custom_user_agent, user_agent&.split(":", 2)&.last&.strip
  end

  private

  # Spins up a local TCP server, yields the port formatted into a URL, captures
  # the raw HTTP/1.1 request headers, then tears down the server.
  def capture_wire_headers(include_values: false)
    require "socket"
    server = TCPServer.new("127.0.0.1", 0)
    port = server.addr[1]
    received = []
    t = Thread.new do
      conn = server.accept
      conn.gets # skip request line
      loop do
        line = conn.gets&.chomp
        break if line.nil? || line.empty?
        received << if include_values
          line
        else
          line.split(":", 2).first.downcase
        end
      end
      conn.write "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
      conn.close
    rescue
      conn&.close
    end
    yield format("http://127.0.0.1:#{port}/")
    t.join(5)
    server.close
    received
  end
end
