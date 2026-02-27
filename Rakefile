# frozen_string_literal: true

require "bundler/gem_tasks"
require "rake/extensiontask"
require "rb_sys"
require "rb_sys/extensiontask"
require "minitest/test_task"

GEMSPEC = Gem::Specification.load("wreq-rb.gemspec")

RbSys::ExtensionTask.new("wreq_rb", GEMSPEC) do |ext|
  ext.lib_dir = "lib/wreq_rb"
  ext.cross_compile = true
  ext.cross_platform = RbSys::ToolchainInfo.supported_ruby_platforms
end

Minitest::TestTask.create(:test) do |t|
  t.libs << "test"
  t.libs << "lib"
  t.test_globs = ["test/**/*_test.rb"]
end

# Apply patches to vendor/wreq before compilation
desc "Apply patches to vendored dependencies"
task :apply_patches do
  patch_dir = File.join(__dir__, "patches")
  wreq_dir = File.join(__dir__, "vendor", "wreq")

  Dir.glob(File.join(patch_dir, "*.patch")).sort.each do |patch|
    # Check if patch is already applied
    check = `cd #{wreq_dir} && git apply --check --reverse #{patch} 2>&1`
    if $?.success?
      puts "Patch already applied: #{File.basename(patch)}"
    else
      puts "Applying patch: #{File.basename(patch)}"
      system("cd #{wreq_dir} && git apply #{patch}") || abort("Failed to apply #{patch}")
    end
  end
end

# Hook patch application before compile — ensure patches are applied
# before the per-platform compile task starts.
Rake::Task["compile"].enhance([:apply_patches]) do; end
Rake::Task["compile"].prerequisites.delete("apply_patches")
Rake::Task["compile"].prerequisites.unshift("apply_patches")

# Reset vendored submodules to clean state on rake clean
task :reset_submodules do
  puts "Resetting vendored submodules..."
  sh "git submodule foreach git reset --hard"
  sh "git submodule foreach git clean -fd"
end
task clean: :reset_submodules

task default: %i[compile test]
