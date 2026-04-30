//! ASP transport layer (Plan 18 Task 5).
//!
//! NDJSON over an arbitrary byte stream. The trait surface is small
//! on purpose — read one request line, write one response line —
//! so the four supported transports (stdio, Unix socket, Windows
//! Named Pipe, WebSocket) all plug in behind the same shape and
//! the verb dispatch / server main loop don't need to care which.
//!
//! Phase 2 (this commit) ships:
//! - The [`Transport`] trait abstraction.
//! - [`stdio::StdioTransport`] — reads from stdin, writes to stdout.
//!   Suitable for the dev-tools agent CLI in `bin/`.
//!
//! Phase 2 follow-ups land the other three transports behind the
//! same trait. The server / dispatch code is decoupled from any
//! one transport so adding e.g. `tokio-tungstenite`-backed
//! WebSocket is purely additive.

#[cfg(feature = "dev-asp")]
pub mod stdio;

#[cfg(feature = "dev-asp")]
pub use stdio::StdioTransport;

/// Transport-layer error. Stringified upstream so verb dispatch
/// can include the failure reason in the audit ring without
/// dragging the underlying `std::io::Error` type through every
/// trait boundary.
#[cfg(feature = "dev-asp")]
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

#[cfg(feature = "dev-asp")]
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Eof => f.write_str("transport reached EOF"),
            TransportError::Io(e) => write!(f, "transport I/O error: {}", e),
        }
    }
}

#[cfg(feature = "dev-asp")]
impl std::error::Error for TransportError {}

/// One line in / one line out. The trait is intentionally
/// synchronous — the ASP server runs on its own thread and blocks
/// on the transport between requests; `async-trait` overhead
/// would buy nothing because we're not multiplexing.
///
/// `read_line` strips the trailing newline; `write_line` adds one.
/// Empty / whitespace-only lines are surfaced unchanged so the
/// verb-dispatch layer can decide whether to error or skip.
#[cfg(feature = "dev-asp")]
pub trait Transport {
    fn read_line(&mut self) -> Result<String, TransportError>;
    fn write_line(&mut self, line: &str) -> Result<(), TransportError>;
}
