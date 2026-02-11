# frozen_string_literal: true

require "bundler/gem_tasks"
require "rake/extensiontask"
require "rb_sys/extensiontask"
require "minitest/test_task"

GEMSPEC = Gem::Specification.load("wreq.gemspec")

RbSys::ExtensionTask.new("wreq_rb", GEMSPEC) do |ext|
  ext.lib_dir = "lib/wreq_rb"
  ext.cross_compile = true
  ext.cross_platform = %w[
    aarch64-linux-musl
    x86_64-linux-musl
    arm64-darwin
    x86_64-darwin
    x64-mingw-ucrt
  ]
end

Minitest::TestTask.create(:test) do |t|
  t.libs << "test"
  t.libs << "lib"
  t.test_globs = ["test/**/*_test.rb"]
end

task default: %i[compile test]
