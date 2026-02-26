# frozen_string_literal: true

require "json"

begin
  # pre-compiled extension by rake-compiler is located inside lib/wreq_rb/<ruby_version>/
  RUBY_VERSION =~ /(\d+\.\d+)/
  require_relative "wreq_rb/#{Regexp.last_match(1)}/wreq_rb"
rescue LoadError => e
  # fallback to the locally built extension
  require_relative 'wreq_rb/wreq_rb'
end

require_relative "wreq-rb/version"

module Wreq
end
