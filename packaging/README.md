# `packaging/` — distribution templates

Per-platform package templates for the `jian` CLI. Plan 9 Task 9
completion: each template has a leading `# TEMPLATE:` comment block
describing every placeholder (`@@VERSION@@`, `@@SHA256_*@@`, etc.)
that a release script substitutes with real values before publishing.

| Path | Channel | Substitutions |
|------|---------|---------------|
| `homebrew/jian.rb` | Homebrew tap (`zseven-w/tap`) | `@@VERSION@@`, `@@SHA256_MAC_ARM@@`, `@@SHA256_LINUX_X86@@`, `@@SHA256_LINUX_ARM@@` (no Intel macOS slot — see formula header) |
| `winget/manifests/jian.installer.yaml` | winget-pkgs (`ZSevenW.Jian`) | `@@VERSION@@`, `@@SHA256_WIN_X86@@` |
| `winget/manifests/jian.locale.en-US.yaml` | winget-pkgs | `@@VERSION@@` |
| `winget/manifests/jian.yaml` | winget-pkgs | `@@VERSION@@` |
| `install.sh` | curl \| sh fallback | none — runtime arch / OS detection |

## How a release uses these

1. Build per-arch binaries via `cargo build --release -p jian` on the
   matrix in `.github/workflows/ci.yml::test` (linux-x86_64,
   linux-aarch64, macos-aarch64, windows-x86_64).
2. `gh release create vX.Y.Z` uploads the four archives; SHA256 sums
   land alongside.
3. A release script (Plan 9 follow-up — not committed here) reads the
   sums, substitutes every `@@…@@` placeholder, and PRs the rendered
   files into the corresponding tap / winget-pkgs / install bucket.
4. `install.sh` lives at a stable URL (e.g. `get.jian.dev/install.sh`)
   so users can run `curl -sSf <url> | sh` without round-tripping the
   release script.

The templates intentionally contain no real version or hash — that's
the release script's job. CI lints them for placeholder hygiene
(no real-looking semver, no real-looking sums).

## Why per-arch SHA256 placeholders for Homebrew

Homebrew formulas resolve `Hardware::CPU.arm?` / `intel?` and
`OS.mac?` / `OS.linux?` at install time and pick the matching URL +
sha256. The single `.rb` ships four `(url, sha256)` pairs.

## Why no Linux x86_64 install.sh on macOS

`install.sh` deliberately exits with `1` on macOS / Windows — Homebrew
and winget are the supported channels there. Forcing a curl-based
install onto macOS would bypass the Cellar / `brew uninstall`
lifecycle that users expect.
