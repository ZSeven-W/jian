//! Unix domain socket transport (Plan 18 ASP prod mode §6 / C4).
//!
//! Local-only by construction — Unix sockets cannot reach the
//! network. The [`UnixSocketListener`] binds the path with the
//! parent directory mode-locked to `0700` and the socket file
//! mode-locked to `0600`, so even on a multi-user box the agent
//! channel is not reachable by another local user. Removing the
//! socket file on drop avoids leaving stale entries that confuse
//! the next run.
//!
//! Each accepted connection becomes one [`UnixSocketTransport`]
//! that wraps the `UnixStream` in the same `Box<dyn BufRead>` /
//! `Box<dyn Write>` shape as [`super::stdio::StdioTransport`], so
//! `run_session` / `run_prod_session` accept either without
//! noticing.

use super::{Transport, TransportError};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Listens on a Unix domain socket and yields per-connection
/// [`UnixSocketTransport`]s. Drops the socket file on `drop` so a
/// crashed run doesn't poison the path for the next one.
pub struct UnixSocketListener {
    listener: UnixListener,
    path: PathBuf,
    /// `true` when this listener created the parent directory and
    /// should remove it on drop. We don't unconditionally `rmdir`
    /// the parent because a co-running peer may have placed its
    /// own socket alongside ours.
    cleanup_parent: bool,
}

impl UnixSocketListener {
    /// Bind a Unix listener at `path`, creating the parent directory
    /// (mode `0700`) if needed. Removes any stale socket file at
    /// `path` first — that's safe because the only legitimate owner
    /// of the path is a previous, now-dead run of this same
    /// process; a hostile peer who can write to a `0700` parent
    /// already has the same uid, in which case race-protection is
    /// moot.
    ///
    /// Returns `TransportError::Io` for any of: parent-dir create
    /// failure, stale-socket-remove failure, bind failure, or perm
    /// chmod failure. The CLI surfaces those verbatim.
    pub fn bind(path: impl AsRef<Path>) -> Result<Self, TransportError> {
        let path = path.as_ref().to_path_buf();
        let cleanup_parent = ensure_parent_dir(&path)?;

        // Remove a stale socket at the same path. `UnixListener::bind`
        // refuses with `EADDRINUSE` if the file already exists, even
        // when no peer is listening — Unix sockets don't auto-clean
        // on process exit. We narrow the cleanup to the exact path
        // and only proceed if the file is a socket.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_socket() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        return Err(TransportError::Io(format!(
                            "stale socket at {} could not be removed: {}",
                            path.display(),
                            e
                        )));
                    }
                } else {
                    // Don't clobber regular files / dirs / symlinks at
                    // the requested path — that's the user's data.
                    return Err(TransportError::Io(format!(
                        "refusing to bind: {} exists and is not a Unix socket",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(TransportError::Io(format!(
                    "stat {}: {}",
                    path.display(),
                    e
                )));
            }
        }

        let listener = UnixListener::bind(&path).map_err(|e| {
            TransportError::Io(format!("bind {}: {}", path.display(), e))
        })?;

        // Pin the socket file to mode `0600`. On most kernels a
        // newly-bound socket inherits the process umask; an
        // 0022-umask process would expose `world-readable` here.
        // `world-read` on a Unix socket doesn't grant connect, but
        // the spec calls out restrictive perms explicitly so we
        // belt-and-braces it.
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).map_err(|e| {
            TransportError::Io(format!("chmod 0600 {}: {}", path.display(), e))
        })?;

        Ok(Self {
            listener,
            path,
            cleanup_parent,
        })
    }

    /// Block until a peer connects, then return a transport bound to
    /// that connection. `accept` is the standard server loop entry
    /// point; the caller decides whether to spawn a thread per
    /// accepted connection or handle one then exit.
    pub fn accept(&self) -> Result<UnixSocketTransport, TransportError> {
        let (stream, _addr) = self
            .listener
            .accept()
            .map_err(|e| TransportError::Io(format!("accept: {}", e)))?;
        UnixSocketTransport::from_stream(stream)
    }

    /// The fully-resolved path the listener is bound to. Useful for
    /// the CLI to print so the agent client knows where to dial.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UnixSocketListener {
    fn drop(&mut self) {
        // Best-effort cleanup. A failure here is informational only —
        // the kernel reaps the listener fd regardless and the next
        // bind() either re-uses or removes the stale entry.
        let _ = std::fs::remove_file(&self.path);
        if self.cleanup_parent {
            if let Some(parent) = self.path.parent() {
                // `remove_dir` only succeeds if the dir is empty; if
                // a sibling listener is still live we leave the
                // parent alone.
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

/// Single-connection transport over an accepted `UnixStream`. The
/// duplicated stream (`try_clone`) gives us independent read/write
/// halves so `read_line` can buffer without aliasing the writer.
pub struct UnixSocketTransport {
    reader: Box<dyn BufRead>,
    writer: Box<dyn Write>,
}

impl UnixSocketTransport {
    fn from_stream(stream: UnixStream) -> Result<Self, TransportError> {
        let writer_stream = stream
            .try_clone()
            .map_err(|e| TransportError::Io(format!("clone unix stream: {}", e)))?;
        Ok(Self {
            reader: Box::new(BufReader::new(stream)),
            writer: Box::new(writer_stream),
        })
    }
}

impl Transport for UnixSocketTransport {
    fn read_line(&mut self) -> Result<String, TransportError> {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) => Err(TransportError::Eof),
            Ok(_) => {
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Ok(buf)
            }
            Err(e) => Err(TransportError::Io(e.to_string())),
        }
    }

    fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| TransportError::Io(e.to_string()))
    }
}

/// Ensure the listener path's parent directory exists with `0700`
/// perms. Returns whether *this call* created the directory; the
/// `Drop` impl uses that flag to decide whether the parent is ours
/// to clean up.
fn ensure_parent_dir(path: &Path) -> Result<bool, TransportError> {
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    if parent.as_os_str().is_empty() {
        // Path was a bare filename in the CWD — no parent to make.
        return Ok(false);
    }
    match std::fs::metadata(parent) {
        Ok(meta) if meta.is_dir() => {
            // Pre-existing dir — leave perms alone (it might be
            // `XDG_RUNTIME_DIR` itself, owned by systemd at 0700).
            // Caller does not own this path; do not clean up.
            Ok(false)
        }
        Ok(_) => Err(TransportError::Io(format!(
            "{} exists and is not a directory",
            parent.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(parent).map_err(|e| {
                TransportError::Io(format!("create_dir_all {}: {}", parent.display(), e))
            })?;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(parent, perms).map_err(|e| {
                TransportError::Io(format!("chmod 0700 {}: {}", parent.display(), e))
            })?;
            Ok(true)
        }
        Err(e) => Err(TransportError::Io(format!(
            "stat {}: {}",
            parent.display(),
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_socket_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jian-asp-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("asp.sock")
    }

    #[test]
    fn bind_creates_socket_with_restrictive_perms() {
        let path = temp_socket_path("perms");
        let listener = UnixSocketListener::bind(&path).expect("bind");

        // Socket file exists and is mode 0600.
        let meta = std::fs::metadata(&path).expect("socket metadata");
        assert!(meta.file_type().is_socket(), "expected socket file");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket mode should be 0600, got 0o{:o}", mode);

        // Parent dir is mode 0700 (created fresh by the listener).
        let parent_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o700);

        drop(listener);
        // After drop, the socket file is gone.
        assert!(!path.exists(), "listener should remove socket on drop");
    }

    #[test]
    fn bind_clears_stale_socket() {
        let path = temp_socket_path("stale");
        // Create the parent dir + a stale socket the way a crashed
        // previous run would leave one behind.
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        {
            let _stale = UnixListener::bind(&path).unwrap();
        }
        // (`_stale` dropped — listener fd is gone but the socket
        // *file* is still on disk, exactly the stale-state case.)
        assert!(path.exists(), "test setup: stale socket should exist");

        let listener = UnixSocketListener::bind(&path).expect("rebind clears stale socket");
        assert!(path.exists());
        drop(listener);
    }

    #[test]
    fn bind_refuses_to_clobber_a_regular_file() {
        let path = temp_socket_path("regfile");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"important user data").unwrap();
        let result = UnixSocketListener::bind(&path);
        let err = match result {
            Ok(_) => panic!("expected refusal, got Ok"),
            Err(e) => e,
        };
        match err {
            TransportError::Io(msg) => assert!(
                msg.contains("not a Unix socket"),
                "expected refusal message, got {}",
                msg
            ),
            other => panic!("expected Io, got {:?}", other),
        }
        // File survived.
        assert!(path.exists());
        // Cleanup so a re-run doesn't trip on the leftover regular
        // file (the listener never owned the parent dir).
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn round_trip_request_response_over_socket() {
        let path = temp_socket_path("round-trip");
        let listener = UnixSocketListener::bind(&path).expect("bind");
        let path_for_client = path.clone();

        // Server thread accepts one connection, echoes one line.
        let server = std::thread::spawn(move || {
            let mut t = listener.accept().expect("accept");
            let line = t.read_line().expect("read");
            assert_eq!(line, r#"{"id":1,"verb":"exit"}"#);
            t.write_line(r#"{"id":1,"ok":true,"body":"bye"}"#)
                .expect("write");
            // keep listener alive until thread joins so the file
            // isn't yanked while the client still has a fd.
        });

        // Client connects + drives one round-trip.
        // Tiny retry to absorb the race where the test lands here
        // before `accept()` has the fd ready in the kernel queue
        // (vanishingly small but observable on busy CI).
        let mut attempts = 0;
        let stream = loop {
            match UnixStream::connect(&path_for_client) {
                Ok(s) => break s,
                Err(_) if attempts < 10 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("client connect: {}", e),
            }
        };
        let mut client = UnixSocketTransport::from_stream(stream).expect("client transport");
        client
            .write_line(r#"{"id":1,"verb":"exit"}"#)
            .expect("client write");
        let resp = client.read_line().expect("client read");
        assert_eq!(resp, r#"{"id":1,"ok":true,"body":"bye"}"#);

        server.join().unwrap();
    }
}
