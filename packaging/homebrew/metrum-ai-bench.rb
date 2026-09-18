# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0

class MetrumAiBench < Formula
  desc "OpenAI-compatible inference load testing and benchmarking tools"
  homepage "https://github.com/metrum-ai/bench-cli"
  version "1.0.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/metrum-ai/bench-cli/releases/download/v#{version}/metrum-ai-bench-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "RELEASE_WORKFLOW_UPDATES_THIS_VALUE"
    else
      url "https://github.com/metrum-ai/bench-cli/releases/download/v#{version}/metrum-ai-bench-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "RELEASE_WORKFLOW_UPDATES_THIS_VALUE"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/metrum-ai/bench-cli/releases/download/v#{version}/metrum-ai-bench-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "RELEASE_WORKFLOW_UPDATES_THIS_VALUE"
    else
      url "https://github.com/metrum-ai/bench-cli/releases/download/v#{version}/metrum-ai-bench-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "RELEASE_WORKFLOW_UPDATES_THIS_VALUE"
    end
  end

  def install
    bin.install Dir["bin/*"]
  end

  test do
    assert_match "Benchmark OpenAI-compatible", shell_output("#{bin}/metrum-ai-bench --help")
    assert_match "Deterministic mock", shell_output("#{bin}/metrum-ai-bench-mock-server --help")
  end
end
