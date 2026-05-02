//! ASP transport layer (Plan 18 Task 5 + Plan 18 ASP prod mode C4).
//!
//! NDJSON over an arbitrary byte stream. The trait surface is small
//! on purpose — read one request line, write one response line —
//! so every supported transport plugs in behind the same shape and
//! the verb dispatch / server main loop don't need to care which.
//!
//! Currently shipped:
//! - [`Transport`] trait abstraction + [`TransportError`].
//! - [`stdio::StdioTransport`] — reads from stdin, writes to stdout.
//!   Used by the dev-tools agent CLI.
//! - [`unix_socket::UnixSocketListener`] /
//!   [`unix_socket::UnixSocketTransport`] — bound to a filesystem
//!   path with `0600` socket / `0700` parent-dir perms, used by
//!   `jian player --asp <path>` on macOS / Linux. Spec §6.
//! - [`named_pipe::NamedPipeListener`] / [`named_pipe::NamedPipeTransport`]
//!   on Windows — `CreateNamedPipeW` + `ConnectNamedPipe` with a
//!   protected DACL granting `GENERIC_ALL` only to the calling
//!   user's resolved SID (no Everyone, no Anonymous). Bound on
//!   `\\.\pipe\jian\<pid>\asp` by `jian player --asp`.
//! - [`socket_path::resolve_bind_arg`] — translates `--asp <arg>`
//!   into a [`socket_path::BindTarget`] and refuses any value that
//!   looks like a network bind (TCP / `host:port` / URL with
//!   scheme), per spec §6.
//!
//! Future-additive: the server / dispatch code is decoupled from
//! any one transport so a `tokio-tungstenite`-backed WebSocket
//! variant could land behind the same trait.

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod stdio;

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub use stdio::StdioTransport;

// `socket_path` is platform-agnostic and only allocates strings —
// safe to expose under either feature gate. The CLI consumes it
// from the `prod-asp` path; dev hosts can use it too if they want
// to bind a Unix socket for an interactive REPL session.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub mod socket_path;

#[cfg(all(unix, any(feature = "dev-asp", feature = "prod-asp")))]
pub mod unix_socket;

#[cfg(all(unix, any(feature = "dev-asp", feature = "prod-asp")))]
pub use unix_socket::{UnixSocketListener, UnixSocketTransport};

#[cfg(all(windows, any(feature = "dev-asp", feature = "prod-asp")))]
pub mod named_pipe;

#[cfg(all(windows, any(feature = "dev-asp", feature = "prod-asp")))]
pub use named_pipe::NamedPipeListener;

/// Transport-layer error. Stringified upstream so verb dispatch
/// can include the failure reason in the audit ring without
/// dragging the underlying `std::io::Error` type through every
/// trait boundary.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
#[derive(Debug)]
pub enum TransportError {
    /// EOF reached before a complete line was read. Some transports
    /// (stdio piped to a file) hit this on legitimate shutdown;
    /// others (sockets) never see it without a peer disconnect.
    Eof,
    /// I/O failure. Carries the underlying error's `Display` form so
    /// the audit log doesn't need to thread an `Arc<dyn Error>`.
    Io(String),
}

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Eof => f.write_str("transport reached EOF"),
            TransportError::Io(e) => write!(f, "transport I/O error: {}", e),
        }
    }
}

#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
impl std::error::Error for TransportError {}

/// One line in / one line out. The trait is intentionally
/// synchronous — the ASP server runs on its own thread and blocks
/// on the transport between requests; `async-trait` overhead
/// would buy nothing because we're not multiplexing.
///
/// `read_line` strips the trailing newline; `write_line` adds one.
/// Empty / whitespace-only lines are surfaced unchanged so the
/// verb-dispatch layer can decide whether to error or skip.
#[cfg(any(feature = "dev-asp", feature = "prod-asp"))]
pub trait Transport {
    fn read_line(&mut self) -> Result<String, TransportError>;
    fn write_line(&mut self, line: &str) -> Result<(), TransportError>;
}
