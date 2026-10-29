#!/usr/bin/env bash
# Prints the Homebrew formula for a vigil release.
#
#   scripts/formula.sh 0.1.0 <dir-with-.sha256-files> > Formula/vigil.rb
#
# The .sha256 files are the ones the release workflow attaches to each
# GitHub release (`gh release download v0.1.0 --pattern '*.sha256'`).
set -euo pipefail

version="${1:?usage: formula.sh VERSION SUMS_DIR}"
sums="${2:?usage: formula.sh VERSION SUMS_DIR}"
base="https://github.com/ElyOgl/vigil/releases/download/v${version}"

sha() {
  local file="$sums/vigil-v${version}-$1.tar.gz.sha256"
  [[ -f "$file" ]] || { echo "missing $file" >&2; exit 1; }
  cut -d' ' -f1 "$file"
}

cat <<EOF
class Vigil < Formula
  desc "Fast, beautiful terminal system monitor (P/E cores, memory pressure, GPU)"
  homepage "https://github.com/ElyOgl/vigil"
  version "${version}"
  license "MIT"

  on_macos do
    on_arm do
      url "${base}/vigil-v${version}-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha aarch64-apple-darwin)"
    end
    on_intel do
      url "${base}/vigil-v${version}-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "${base}/vigil-v${version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$(sha aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "${base}/vigil-v${version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$(sha x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "vigil"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/vigil --version")
    assert_match "logical_cores", shell_output("#{bin}/vigil --json --top 1")
  end
end
EOF
