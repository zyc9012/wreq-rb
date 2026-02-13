# frozen_string_literal: true

require_relative "test_helper"

class ThreadSafetyTest < Minitest::Test
  def test_gvl_released_during_network_io
    # If the GVL is properly released, multiple threads can make
    # requests concurrently and finish faster than sequentially.
    client = Wreq::Client.new(timeout: 10)
    num_threads = 4
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

    results.each { |status| assert_equal 200, status }

    # If GVL were held, this would take ~num_threads * delay seconds.
    # With GVL released, requests run in parallel: ~delay seconds + overhead.
    max_expected = delay * num_threads - 1
    assert elapsed < max_expected,
      "Expected concurrent requests to finish in <#{max_expected}s, " \
      "but took #{elapsed.round(2)}s (GVL may not be released)"
  end

  def test_ruby_thread_runs_during_request
    # Verify that a Ruby thread can do meaningful work while another
    # thread is blocked on network I/O in native code.
    client = Wreq::Client.new(timeout: 10)
    counter = 0
    stop = false

    request_thread = Thread.new do
      client.get("https://httpbin.org/delay/2")
    end

    # Let the request thread start and enter the native blocking call
    sleep 0.3

    counter_thread = Thread.new do
      until stop
        counter += 1
        # Yield to let other threads run; this is pure Ruby, needs GVL
        Thread.pass
      end
    end

    # Wait for the request to finish
    request_thread.join
    stop = true
    counter_thread.join

    assert counter > 1000,
      "Ruby thread only incremented counter #{counter} times during a 2s request " \
      "(expected >1000 if GVL is released)"
  end
end
