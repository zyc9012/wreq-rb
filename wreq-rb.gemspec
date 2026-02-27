# frozen_string_literal: true

require_relative "lib/wreq-rb/version"

Gem::Specification.new do |spec|
  spec.name          = "wreq-rb"
  spec.version       = Wreq::VERSION
  spec.authors       = ["Yicheng Zhou", "Illia Zub"]
  spec.summary       = "Ruby HTTP client featuring TLS fingerprint emulation, HTTP/2 support, cookie handling, and proxy support."
  spec.description   = "An ergonomic Ruby HTTP client powered by Rust's wreq library, " \
                        "featuring TLS fingerprint emulation (JA3/JA4), HTTP/2 support, " \
                        "cookie handling, proxy support, and redirect policies."
  spec.homepage      = "https://github.com/serpapi/wreq-rb"
  spec.license       = "MIT"
  spec.required_ruby_version = ">= 2.7.0"

  spec.files = Dir[
    "lib/**/*.rb",
    "ext/**/*.{rs,toml,lock,rb}",
    "vendor/wreq/**/*.{rs,toml}",
    "vendor/wreq/LICENSE",
    "vendor/wreq/README.md",
    "patches/*.patch",
    "Cargo.toml",
    "Cargo.lock",
    "LICENSE",
    "README.md",
  ]

  spec.require_paths = ["lib"]
  spec.extensions    = ["ext/wreq_rb/extconf.rb"]

  # 0.9.123 is the last version that uses rake-compiler-dock 1.10.0 (Ruby 2.7 support)
  spec.add_dependency "rb_sys", "0.9.123"
end
