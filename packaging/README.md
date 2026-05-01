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

## Plan 8 §T10 packaging configs (Plan 19 sprint completion)

The Plan-9 install templates above target the **portable archive**
distribution channel (Homebrew, winget, install.sh). Plan 8 §T10
adds **native installer** configs for shipping `.app` / `.msi` / `.deb`
/ `.AppImage` artifacts:

| Path | Channel | Substitutions |
|------|---------|---------------|
| `macos/Info.plist.in` | `cargo bundle --release` (.app) | `@@VERSION@@`, `@@SUFEED_URL@@`, `@@SU_PUBLIC_ED_KEY@@` |
| `macos/generate-icns.sh` | macOS icon-set generator | source PNG path |
| `macos/appcast.xml.tmpl` | Sparkle update feed | `@@VERSION@@`, `@@VERSION_BUILD@@`, `@@PUB_DATE@@`, `@@DOWNLOAD_URL@@`, `@@DOWNLOAD_LENGTH@@`, `@@EDDSA_SIGNATURE@@`, `@@RELEASE_NOTES_URL@@`, `@@MINIMUM_SYSTEM_VERSION@@` |
| `windows/wix/main.wxs.tmpl` | `cargo wix` (MSI) | `@@VERSION@@`, `@@UPGRADE_GUID@@` |
| `linux/jian.desktop` | `.desktop` launcher entry | none |
| `linux/jian.mime.xml` | shared-mime-info `.op` registration | none |
| `linux/install-mime.sh` | post-install xdg-mime registration | `--system` flag |
| `linux/build-appimage.sh` | AppImage build (linuxdeploy) | `APPIMAGE_UPDATE_URL` env var |
| `icon/AppIcon-source.png` | 1024×1024 source for every platform's icon set | none — source asset |

`cargo bundle` / `cargo wix` / `cargo deb` metadata lives in
`crates/jian-cli/Cargo.toml` under `[package.metadata.bundle]`,
`[package.metadata.wix]`, and `[package.metadata.deb]` respectively.

### Per-platform CI verification status

| Task | Status | Verifies on |
|------|--------|-------------|
| C1 macOS .app bundle | ⏳ config shipped | macos-aarch64 release runner |
| C2 Windows MSI       | ⏳ config shipped | windows-x86_64 release runner |
| C3 Linux AppImage + .deb | ⏳ config shipped | linux-x86_64 release runner |
| C4 App icons (.icns / .ico / .desktop) | ⏳ generator + .desktop shipped | macos / windows / linux release runners |
| C5 macOS deep link receiver | ⏳ scaffolding shipped | macos-aarch64 (Apple-Event injection test) |
| C6 Windows deep link receiver | ⏳ scaffolding shipped | windows-x86_64 (`WM_COPYDATA` injection test) |
| C7 Linux MIME install | ⏳ script shipped | linux-x86_64 (.deb install + xdg-mime query) |
| C8 macOS Sparkle | ⏳ Info.plist + appcast template shipped | macos-aarch64 release with signing key |
| C9 Windows selfupdate | ✅ feature-flagged dep on `self_update` already; activates with `--features updater` | windows-x86_64 |
| C10 Linux AppImageUpdate | ⏳ script shipped | linux-x86_64 (zsync feed bucket) |

The Apple-Event receiver and `WM_COPYDATA` window-class plumbing
land in per-platform CI follow-ups — `app_delegate.rs` /
`win_deeplink.rs` ship the testable trait-routing seam today, and
the OS-level message-loop integration drops in alongside the
release-runner CI that can actually exercise it.

## Why per-arch SHA256 placeholders for Homebrew

Homebrew formulas resolve `Hardware::CPU.arm?` / `intel?` and
`OS.mac?` / `OS.linux?` at install time and pick the matching URL +
sha256. The single `.rb` ships four `(url, sha256)` pairs.

## Why no Linux x86_64 install.sh on macOS

`install.sh` deliberately exits with `1` on macOS / Windows — Homebrew
and winget are the supported channels there. Forcing a curl-based
install onto macOS would bypass the Cellar / `brew uninstall`
lifecycle that users expect.
