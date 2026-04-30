# TEMPLATE — Homebrew formula for the `jian` CLI.
#
# Placeholders the release script substitutes:
#   @@VERSION@@           upstream semver, e.g. `0.0.1`
#   @@SHA256_MAC_ARM@@    sha256 of jian-@@VERSION@@-aarch64-apple-darwin.tar.gz
#   @@SHA256_LINUX_X86@@  sha256 of jian-@@VERSION@@-x86_64-unknown-linux-gnu.tar.gz
#   @@SHA256_LINUX_ARM@@  sha256 of jian-@@VERSION@@-aarch64-unknown-linux-gnu.tar.gz
#
# Tap: zseven-w/tap (separate repo `homebrew-tap`). Install via:
#   brew install zseven-w/tap/jian
#
# `on_intel` macOS is intentionally not advertised — the release CI
# matrix in `.github/workflows/release.yml` builds aarch64-apple-darwin
# only, matching the live tier-1 target. Intel Macs install via the
# Linux Rosetta path or build from source. Add `on_intel` here once an
# `x86_64-apple-darwin` runner joins the CI matrix and the release
# script gains `@@SHA256_MAC_X86@@` substitution alongside it.
#
# Bottle blocks intentionally omitted — the prebuilt binaries are
# already arch-specific so a `brew install` just downloads + extracts
# without compilation.

class Jian < Formula
  desc "Jian runtime CLI — check, pack, and run .op files"
  homepage "https://github.com/ZSeven-W/jian"
  version "@@VERSION@@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/ZSeven-W/jian/releases/download/v@@VERSION@@/jian-@@VERSION@@-aarch64-apple-darwin.tar.gz"
      sha256 "@@SHA256_MAC_ARM@@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/ZSeven-W/jian/releases/download/v@@VERSION@@/jian-@@VERSION@@-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@@SHA256_LINUX_ARM@@"
    end
    on_intel do
      url "https://github.com/ZSeven-W/jian/releases/download/v@@VERSION@@/jian-@@VERSION@@-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@@SHA256_LINUX_X86@@"
    end
  end

  def install
    bin.install "jian"
  end

  test do
    # Smoke test: --version prints the upstream semver back.
    assert_match version.to_s, shell_output("#{bin}/jian --version")
  end
end
