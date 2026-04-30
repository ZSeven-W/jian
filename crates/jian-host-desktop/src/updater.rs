//! Auto-updater abstractions (Plan 8 Task 9 scaffolding).
//!
//! Per-platform backends — Sparkle on macOS, the `selfupdate` crate on
//! Windows, AppImageUpdate on Linux — each pull a substantial
//! dependency and need a signed feed URL the workspace doesn't yet
//! own. What ships today is the runtime-side abstraction every
//! backend will plug into:
//!
//! - [`UpdateInfo`] — what the backend's `check` returns: the new
//!   version, a release-notes URL, and a download URL the apply step
//!   uses.
//! - [`Updater`] — the trait every backend implements (`check` is
//!   non-blocking; `apply` is allowed to be blocking and may not
//!   return on success because some backends relaunch the app
//!   directly).
//! - [`NullUpdater`] — no-op default that always reports "up to
//!   date". Hosts that haven't wired a real backend ship this so the
//!   `Updater` trait surface stays uniform across configurations.
//!
//! Per-platform backends (Sparkle / selfupdate / AppImageUpdate)
//! land in dedicated follow-up commits behind their respective
//! `cfg(target_os)` and feature flags.

/// Result of a successful update probe. Returned by [`Updater::check`]
/// when a newer version is available; `None` means "current version
/// is up to date" or "backend has no opinion" (the null impl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Semver version of the candidate update.
    pub version: String,
    /// URL the user can read for release notes / changelog. Optional
    /// because not every backend provides one.
    pub release_notes_url: Option<String>,
    /// URL the apply step should download. Some backends (Sparkle)
    /// store this internally and don't expose it; for those the
    /// field carries the appcast item URL or similar identifier and
    /// `apply` interprets it.
    pub download_url: String,
}

/// Errors an `Updater` can return. Both `check` and `apply` use this.
#[derive(Debug, Clone)]
pub enum UpdaterError {
    /// Network / IO failure during the check or download step. The
    /// string carries the platform backend's own error message —
    /// callers should log it but typically not parse it.
    Io(String),
    /// The release feed parsed but signalled "no update available".
    /// `apply` returns this when called against a stale `UpdateInfo`
    /// whose download has since been retracted.
    NotAvailable,
    /// The backend isn't available on this platform / build (e.g.
    /// `Sparkle` impl invoked on Linux). Hosts can fall back to
    /// [`NullUpdater`] when constructing.
    Unsupported(&'static str),
}

impl std::fmt::Display for UpdaterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdaterError::Io(msg) => write!(f, "updater I/O error: {msg}"),
            UpdaterError::NotAvailable => f.write_str("no update available"),
            UpdaterError::Unsupported(reason) => {
                write!(f, "updater unsupported on this platform: {reason}")
            }
        }
    }
}

impl std::error::Error for UpdaterError {}

/// Auto-updater interface. The trait is deliberately small so each
/// platform backend can implement it without forcing a shared async
/// runtime: `check` returns `Result<Option<_>, _>` synchronously
/// (backends that need network IO push that work onto a worker
/// thread internally), and `apply` is allowed to be slow / blocking.
///
/// `apply`'s success path may not return: Sparkle, for example,
/// relaunches the app directly after install. Implementations document
/// their relaunch behaviour. A returned `Ok(())` means "apply
/// completed; the caller may continue running the current process".
///
/// ### Threading & sharing
///
/// Both methods take `&self`, so stateful backends (e.g. caching the
/// last `UpdateInfo` between `check` and `apply`) need interior
/// mutability — `RefCell` for main-thread-only backends, `Mutex` for
/// shared backends. The trait does not require `Send + Sync`: most
/// updater work runs on the main thread (the user clicked "Check for
/// Updates…" in the menu) and synchronous network probes inside
/// `check` are routine. Hosts that want to share an updater across
/// threads add the bound at their use site
/// (`fn install(updater: Arc<dyn Updater + Send + Sync>)`).
pub trait Updater {
    /// Check the release feed for a newer version. `Ok(None)` means
    /// the running version is current; `Ok(Some(info))` means an
    /// update is available.
    fn check(&self) -> Result<Option<UpdateInfo>, UpdaterError>;

    /// Download + install the update described by `info`. Returns
    /// `Ok(())` if the update was applied without relaunching the
    /// process, or never returns (the backend relaunches the app).
    /// `NotAvailable` is the right error when `info` was constructed
    /// from a check that has since been invalidated.
    fn apply(&self, info: &UpdateInfo) -> Result<(), UpdaterError>;
}

/// GitHub-Releases-backed updater over the `self_update` crate.
/// Available under the `updater` cargo feature. Works on macOS,
/// Windows, and Linux without per-platform fanout — `self_update`
/// detects the host arch + OS and picks the matching release asset
/// by file-extension convention (`*-aarch64-apple-darwin.tar.gz`,
/// `*-x86_64-pc-windows-msvc.zip`, etc — the same names produced by
/// `packaging/install.sh` and the Homebrew formula).
///
/// `check` invokes `self_update::backends::github::ReleaseList::fetch`
/// synchronously (≤ a few seconds on a healthy network). `apply`
/// downloads the chosen release into a temp dir, validates it, then
/// atomically swaps the running binary. **Both calls block the
/// invoking thread** — `apply` in particular can take tens of seconds
/// over a slow link. Hosts MUST dispatch them onto a worker thread
/// (e.g. `std::thread::spawn`) before invoking from a winit
/// callback; doing it inline freezes the event loop. The trait's
/// `&self` receiver makes spawning ergonomic when the updater is
/// `Arc<dyn Updater + Send + Sync>`.
///
/// `apply` is configured with `no_confirm(true)` and
/// `show_output(false)` so it never prompts on stdin or writes
/// progress to stdout — desktop hosts surface progress + confirm
/// dialogs through their own [`crate::services::feedback`] flow.
/// The host *does not* relaunch the app automatically — `apply`
/// returns `Ok(())` and the caller decides when to terminate
/// (typically: prompt the user via the feedback service, then
/// `event_loop.exit()`).
///
/// **Tag convention**: this backend assumes GitHub tags use the
/// `v<semver>` form (matching `packaging/install.sh` + the Homebrew
/// template). `Release.version` from `self_update` strips the `v`,
/// so we re-add it whenever we need the canonical tag. If your
/// release pipeline uses bare `0.1.0` tags instead, override via
/// the `Release` struct directly (todo: configurable prefix).
///
/// Sparkle / AppImageUpdate impls remain deferred — they offer
/// per-platform polish (Sparkle's signed appcast XML, AppImageUpdate's
/// delta updates) but introduce platform fanout this generic backend
/// avoids.
#[cfg(feature = "updater")]
pub struct GitHubReleasesUpdater {
    /// `owner` half of `<owner>/<repo>` on github.com.
    owner: String,
    /// `repo` half of `<owner>/<repo>`.
    repo: String,
    /// Currently-running app version (typically `env!("CARGO_PKG_VERSION")`).
    /// `check` skips returning an `UpdateInfo` when the latest tag's
    /// version isn't strictly greater than this.
    current_version: String,
    /// Filename component to match against release-asset names.
    /// Common values: `"aarch64-apple-darwin"`, `"x86_64-unknown-linux-gnu"`,
    /// `"x86_64-pc-windows-msvc"`. Defaults to a value derived from
    /// `std::env::consts::ARCH` + `OS`.
    target_substring: String,
    /// Name of the binary to swap inside the archive. Defaults to
    /// `"jian"` (matches `packaging/install.sh`'s expectation).
    bin_name: String,
}

#[cfg(feature = "updater")]
impl GitHubReleasesUpdater {
    /// Build with sensible defaults for the running host:
    /// - `current_version` = `CARGO_PKG_VERSION` of the embedding crate
    /// - `target_substring` = canonical `<arch>-<vendor>-<os>` triple
    /// - `bin_name` = `"jian"`
    pub fn new(
        owner: impl Into<String>,
        repo: impl Into<String>,
        current_version: impl Into<String>,
    ) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
            current_version: current_version.into(),
            target_substring: default_target_substring(),
            bin_name: "jian".into(),
        }
    }

    /// Override the asset-filename match. Use this when a release
    /// uses a non-default arch / OS naming convention (e.g.
    /// `linux` instead of `unknown-linux-gnu`).
    pub fn with_target_substring(mut self, target: impl Into<String>) -> Self {
        self.target_substring = target.into();
        self
    }

    /// Override the binary name swapped inside the archive. Default
    /// is `"jian"`; openpencil-shell hosts override to their app
    /// binary name.
    pub fn with_bin_name(mut self, name: impl Into<String>) -> Self {
        self.bin_name = name.into();
        self
    }
}

#[cfg(feature = "updater")]
impl Updater for GitHubReleasesUpdater {
    fn check(&self) -> Result<Option<UpdateInfo>, UpdaterError> {
        // ReleaseList::fetch is a blocking HTTP call to the GitHub API.
        // We map any error path through Io with the underlying message.
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(&self.owner)
            .repo_name(&self.repo)
            .build()
            .and_then(|r| r.fetch())
            .map_err(|e| UpdaterError::Io(e.to_string()))?;
        let Some(latest) = releases.first() else {
            return Ok(None);
        };
        let latest_v = latest.version.trim_start_matches('v');
        let current_v = self.current_version.trim_start_matches('v');
        if !is_strictly_newer(latest_v, current_v) {
            return Ok(None);
        }
        let asset = latest
            .asset_for(&self.target_substring, None)
            .ok_or_else(|| {
                UpdaterError::Io(format!(
                    "release {} has no asset matching `{}`",
                    latest.version, self.target_substring
                ))
            })?;
        // `Release.version` is stripped of the leading `v`; the
        // GitHub tag itself keeps the prefix per our release pipeline
        // convention. Both `release_notes_url` and `apply` need the
        // tag form, not the stripped version, so build it once here.
        let tag = format!("v{}", latest.version);
        Ok(Some(UpdateInfo {
            version: latest.version.clone(),
            release_notes_url: Some(format!(
                "https://github.com/{}/{}/releases/tag/{}",
                self.owner, self.repo, tag,
            )),
            download_url: asset.download_url,
        }))
    }

    fn apply(&self, info: &UpdateInfo) -> Result<(), UpdaterError> {
        // self_update's Update API rebuilds the asset URL internally
        // from `target` + `bin_name` + the discovered version, so we
        // hand it back the same coordinates and let it work.
        // `target_version_tag` MUST be the literal GitHub tag — our
        // pipeline uses `v<semver>`, so re-add the prefix that
        // `Release.version` stripped during `check`.
        // `no_confirm(true)` + `show_output(false)` keep the call
        // headless: no stdin prompt, no progress writes to stdout.
        // Desktop hosts surface UI through `services::feedback`
        // separately.
        let tag = format!("v{}", info.version.trim_start_matches('v'));
        self_update::backends::github::Update::configure()
            .repo_owner(&self.owner)
            .repo_name(&self.repo)
            .bin_name(&self.bin_name)
            .target(&self.target_substring)
            .current_version(self.current_version.trim_start_matches('v'))
            .target_version_tag(&tag)
            .show_download_progress(false)
            .show_output(false)
            .no_confirm(true)
            .build()
            .and_then(|u| u.update())
            .map_err(|e| UpdaterError::Io(e.to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "updater")]
fn default_target_substring() -> String {
    use std::env::consts;
    // Match `target_triple()` from `self_update` minus the
    // libc-suffix variations. Linux glibc reports `gnu`, Windows MSVC
    // reports `msvc`, macOS reports the apple-darwin triple.
    let arch = consts::ARCH;
    match consts::OS {
        "macos" => format!("{}-apple-darwin", arch),
        "linux" => format!("{}-unknown-linux-gnu", arch),
        "windows" => format!("{}-pc-windows-msvc", arch),
        other => format!("{}-{}", arch, other),
    }
}

/// Strictly-greater comparison via `semver::Version`. Inputs that
/// fail to parse — typically because the host author wired an
/// unconventional tag like `release-2026-04-28` — fall back to
/// `false` (the safe default; refuse to auto-update from an
/// unparseable advertised version rather than crashing).
///
/// Honours full semver semantics including prerelease ordering, so
/// `1.0.0-rc.1 < 1.0.0 < 1.0.1`. The previous hand-rolled split-
/// on-dot path mapped non-numeric components to `0`, which mis-
/// ordered `1.0.0-rc.1` as *newer* than `1.0.0` and could
/// auto-roll users from a stable release back onto a release
/// candidate. Pinned in `is_strictly_newer_handles_prerelease`.
#[cfg(feature = "updater")]
fn is_strictly_newer(latest: &str, current: &str) -> bool {
    use semver::Version;
    let parse = |s: &str| Version::parse(s.trim_start_matches('v')).ok();
    match (parse(latest), parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Translate a schema-side [`jian_ops_schema::app::UpdaterConfig`] into
/// a concrete `Box<dyn Updater>`. `None` means **no updater wired**:
/// the caller leaves `DesktopHost::updater` at its default (`None`),
/// which a `MenuHandler` can detect via `host.updater.is_none()`
/// before trying to dispatch `app.check_updates`.
///
/// Returns `Some(...)` only for a kind / param combo that produces a
/// genuinely useful backend. Mappings:
///
/// - `kind: "disabled"` → `None` (explicit opt-out — surfacing it as
///   `Some(NullUpdater)` would force `MenuHandler` authors to also
///   special-case the null case, drift-prone).
/// - `kind: "github_releases"` (feature `updater`, with valid
///   `params.owner` + `params.repo`) → `Some(GitHubReleasesUpdater)`.
///   Missing owner/repo or feature-off → `None` plus a stderr warning
///   explaining the fallback.
/// - Any other `kind` → `None` plus a stderr warning. Third-party
///   hosts that ship their own backends should inspect `cfg.kind`
///   themselves before falling back to this function.
///
/// `current_version` is the running app's semver (typically
/// `env!("CARGO_PKG_VERSION")`); `default_bin_name` is the host's
/// fallback binary name when the schema doesn't override
/// `params.binName`.
pub fn build_updater_from_schema(
    cfg: &jian_ops_schema::app::UpdaterConfig,
    current_version: &str,
    default_bin_name: &str,
) -> Option<Box<dyn Updater>> {
    use jian_ops_schema::app::UpdaterConfig;
    let _ = default_bin_name; // touched only inside the `updater` cfg
    let _ = current_version; //  ditto
    match cfg.kind.as_str() {
        UpdaterConfig::KIND_DISABLED => None,
        #[cfg(feature = "updater")]
        UpdaterConfig::KIND_GITHUB_RELEASES => {
            let owner = cfg.params.get("owner").and_then(|v| v.as_str());
            let repo = cfg.params.get("repo").and_then(|v| v.as_str());
            let target = cfg.params.get("target").and_then(|v| v.as_str());
            // Try both camelCase (`binName`) and snake_case (`bin_name`)
            // so authors using either convention work — the schema
            // defaults to camelCase but JSON tooling sometimes
            // emits snake.
            let bin_name = cfg
                .params
                .get("binName")
                .or_else(|| cfg.params.get("bin_name"))
                .and_then(|v| v.as_str())
                .unwrap_or(default_bin_name);
            match (owner, repo) {
                (Some(o), Some(r)) => {
                    let mut u =
                        GitHubReleasesUpdater::new(o, r, current_version).with_bin_name(bin_name);
                    if let Some(t) = target {
                        u = u.with_target_substring(t);
                    }
                    Some(Box::new(u))
                }
                _ => {
                    eprintln!(
                        "jian-host-desktop: app.updater kind=\"github_releases\" missing owner/repo; updater stays unwired"
                    );
                    None
                }
            }
        }
        #[cfg(not(feature = "updater"))]
        UpdaterConfig::KIND_GITHUB_RELEASES => {
            eprintln!(
                "jian-host-desktop: app.updater kind=\"github_releases\" requires `--features updater`; updater stays unwired"
            );
            None
        }
        other => {
            eprintln!(
                "jian-host-desktop: app.updater kind=\"{}\" not built into jian-host-desktop; updater stays unwired",
                other
            );
            None
        }
    }
}

/// No-op updater: reports "no update available" and refuses to apply.
/// The default for hosts without a real backend wired up.
#[derive(Debug, Default, Copy, Clone)]
pub struct NullUpdater;

impl Updater for NullUpdater {
    fn check(&self) -> Result<Option<UpdateInfo>, UpdaterError> {
        Ok(None)
    }

    fn apply(&self, _info: &UpdateInfo) -> Result<(), UpdaterError> {
        Err(UpdaterError::Unsupported("no updater backend wired"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_updater_check_returns_none() {
        let u = NullUpdater;
        assert_eq!(u.check().unwrap(), None);
    }

    #[test]
    fn null_updater_apply_returns_unsupported() {
        let u = NullUpdater;
        let info = UpdateInfo {
            version: "1.2.3".into(),
            release_notes_url: None,
            download_url: "https://example.com/x.dmg".into(),
        };
        match u.apply(&info) {
            Err(UpdaterError::Unsupported(_)) => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn update_info_round_trips_clone_and_eq() {
        let info = UpdateInfo {
            version: "0.0.2".into(),
            release_notes_url: Some("https://example.com/notes".into()),
            download_url: "https://example.com/x.dmg".into(),
        };
        assert_eq!(info, info.clone());
    }

    #[cfg(feature = "updater")]
    #[test]
    fn is_strictly_newer_handles_basic_semver() {
        use super::is_strictly_newer;
        assert!(is_strictly_newer("0.0.2", "0.0.1"));
        assert!(is_strictly_newer("1.0.0", "0.9.9"));
        assert!(is_strictly_newer("0.1.0", "0.0.99"));
        assert!(!is_strictly_newer("0.0.1", "0.0.1"));
        assert!(!is_strictly_newer("0.0.1", "0.0.2"));
    }

    #[cfg(feature = "updater")]
    #[test]
    fn is_strictly_newer_handles_prerelease() {
        use super::is_strictly_newer;
        // Prereleases come BEFORE the bare release per semver §11.
        // The pre-fix path mis-mapped non-numeric components to 0
        // and treated `1.0.0-rc.1` as newer than `1.0.0`, which
        // could roll users from a stable release back onto a
        // release candidate.
        assert!(!is_strictly_newer("1.0.0-rc.1", "1.0.0"));
        assert!(is_strictly_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_strictly_newer("1.0.0-rc.2", "1.0.0-rc.1"));
        assert!(!is_strictly_newer("1.0.0-rc.1", "1.0.0-rc.2"));
    }

    #[cfg(feature = "updater")]
    #[test]
    fn is_strictly_newer_tolerates_v_prefix() {
        use super::is_strictly_newer;
        // GitHub `Release.version` strips `v`, but defensive callers
        // may pass either form. Both should compare correctly.
        assert!(is_strictly_newer("v0.0.2", "0.0.1"));
        assert!(is_strictly_newer("0.0.2", "v0.0.1"));
        assert!(is_strictly_newer("v0.0.2", "v0.0.1"));
    }

    #[cfg(feature = "updater")]
    #[test]
    fn is_strictly_newer_unparseable_returns_false() {
        use super::is_strictly_newer;
        // Refuse to auto-update from an unparseable version rather
        // than guessing — the alternative is silently rolling users
        // forward on a release tag the script doesn't understand.
        assert!(!is_strictly_newer("release-2026-04-28", "0.0.1"));
        assert!(!is_strictly_newer("0.0.1", "wat"));
    }

    #[cfg(feature = "updater")]
    #[test]
    fn default_target_substring_includes_platform_marker() {
        use super::default_target_substring;
        let s = default_target_substring();
        // Don't pin the exact arch — CI runs on multiple — just
        // confirm the OS suffix follows the Rust target-triple
        // convention every release expects.
        let ok = s.contains("apple-darwin")
            || s.contains("unknown-linux-gnu")
            || s.contains("pc-windows-msvc");
        assert!(ok, "unexpected target triple: {}", s);
    }

    #[test]
    fn updater_error_display_includes_message() {
        let e = UpdaterError::Io("connection refused".into());
        assert!(e.to_string().contains("connection refused"));

        let e = UpdaterError::Unsupported("Sparkle on Linux");
        assert!(e.to_string().contains("Sparkle on Linux"));

        assert_eq!(
            UpdaterError::NotAvailable.to_string(),
            "no update available"
        );
    }

    /// Demonstrates the canonical custom-impl shape that future
    /// platform backends (Sparkle / selfupdate / AppImageUpdate)
    /// will follow: stash a candidate `UpdateInfo` and serve it from
    /// `check`; `apply` confirms the version line up.
    #[test]
    fn custom_updater_returns_candidate() {
        struct Stub {
            candidate: UpdateInfo,
        }
        impl Updater for Stub {
            fn check(&self) -> Result<Option<UpdateInfo>, UpdaterError> {
                Ok(Some(self.candidate.clone()))
            }
            fn apply(&self, info: &UpdateInfo) -> Result<(), UpdaterError> {
                if info == &self.candidate {
                    Ok(())
                } else {
                    Err(UpdaterError::NotAvailable)
                }
            }
        }
        let stub = Stub {
            candidate: UpdateInfo {
                version: "0.0.2".into(),
                release_notes_url: None,
                download_url: "x".into(),
            },
        };
        let info = stub.check().unwrap().unwrap();
        assert_eq!(info.version, "0.0.2");
        stub.apply(&info)
            .expect("apply succeeds with matching info");

        let stale = UpdateInfo {
            version: "0.0.1".into(),
            release_notes_url: None,
            download_url: "x".into(),
        };
        assert!(matches!(
            stub.apply(&stale),
            Err(UpdaterError::NotAvailable)
        ));
    }
}
