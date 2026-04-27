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
