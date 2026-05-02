//! Windows Named Pipe transport stub (Plan 18 ASP prod mode §6 / C4).
//!
//! Per spec §6, Windows prod ASP should bind a per-process Named
//! Pipe at `\\.\pipe\jian\<pid>\asp` with a current-user-only ACL.
//! `std` does not expose Named Pipes — a real implementation needs
//! `windows-sys` (or `tokio` for async). The workspace doesn't pull
//! either today, so this commit ships the path resolution + the
//! Transport-level shape and defers the binding to a follow-up
//! commit.
//!
//! Calling [`NamedPipeListener::bind`] returns `TransportError::Io`
//! with a clearly-marked "not yet implemented" message so the CLI's
//! `--asp` flag surfaces a useful error on Windows instead of
//! silently exiting. Tests on Unix don't compile this module.

use super::TransportError;

/// Windows Named Pipe listener. Currently a typed stub — see the
/// module docs.
pub struct NamedPipeListener {
    /// `\\.\pipe\jian\<pid>\asp` or whatever the resolver chose.
    /// Stored so that, once the real implementation lands, the
    /// listener can advertise the path the same way the Unix one
    /// does via `path()`.
    #[allow(dead_code)]
    name: String,
}

impl NamedPipeListener {
    pub fn bind(name: impl Into<String>) -> Result<Self, TransportError> {
        let name = name.into();
        Err(TransportError::Io(format!(
            "Windows Named Pipe transport not yet implemented \
             (would bind `{}`). Tracking issue: Plan 18 C4 \
             follow-up — needs `windows-sys` (DACL + \
             CreateNamedPipeW + ConnectNamedPipe).",
            name
        )))
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
