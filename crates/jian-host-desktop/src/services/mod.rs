//! Desktop-host platform services.
//!
//! MVP ships in-memory implementations so the desktop host can run
//! deterministically without depending on SQLite / reqwest / OS
//! clipboard at build time. Plan 10+ swaps these for real backends
//! (`rusqlite` storage, `reqwest` network, `arboard` clipboard —
//! already feature-gated in `Cargo.toml`).

pub mod router;
pub mod storage;

#[cfg(feature = "clipboard")]
pub mod clipboard;

#[cfg(feature = "network")]
pub mod network;

#[cfg(feature = "feedback")]
pub mod feedback;

pub use router::HistoryRouter;
pub use storage::InMemoryStorage;

#[cfg(feature = "network")]
pub use network::DesktopNetworkClient;

#[cfg(feature = "feedback")]
pub use feedback::DesktopFeedback;
