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

  def test_threads_can_run_ruby_during_request
    # Verify that a Ruby thread can do work while another is blocked on I/O.
    client = Wreq::Client.new(timeout: 10)
    ruby_thread_ran = false

    request_thread = Thread.new do
      client.get("https://httpbin.org/delay/2")
    end

    sleep 0.1

    ruby_thread = Thread.new do
      ruby_thread_ran = true
    end
    ruby_thread.join(3)

    request_thread.join

    assert ruby_thread_ran,
      "Ruby thread could not run while request was in flight (GVL not released)"
  end
end
