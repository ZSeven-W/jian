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
use std::fs::DirBuilder;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
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
    /// (mode `0700`) if needed.
    ///
    /// Security:
    /// - Parent directory is created with `mkdir(parent, 0o700)` —
    ///   one syscall, atomic, no chmod-after race.
    /// - A pre-existing parent must be owned by the current uid AND
    ///   have mode bits `& 0o077 == 0` (no group/other access). If
    ///   either check fails the bind is refused — the spec says the
    ///   transport is local-only-and-confidential, so binding inside
    ///   a world-readable parent would silently weaken that.
    /// - A stale socket at `path` is unlinked **only after** probing
    ///   that no one is listening on it (`UnixStream::connect`
    ///   returns `ECONNREFUSED` for a listener-less socket file). A
    ///   live listener belonging to a co-running peer is left
    ///   untouched and the bind fails — that's safer than silently
    ///   stealing the path.
    /// - Socket file is chmodded to `0600` after bind. The chmod
    ///   window is harmless because the parent is `0700` (only the
    ///   owner can traverse to the socket file in the first place),
    ///   but we tighten the file perms regardless so the path is
    ///   inert if it ever gets snapshotted by a backup tool.
    pub fn bind(path: impl AsRef<Path>) -> Result<Self, TransportError> {
        let path = path.as_ref().to_path_buf();
        let cleanup_parent = ensure_parent_dir(&path)?;

        // Stale-socket handling. `UnixListener::bind` refuses with
        // `EADDRINUSE` when the file already exists, even when no
        // peer is listening — Unix sockets don't auto-clean on
        // process exit. We probe before unlinking so a still-live
        // co-running listener isn't silently displaced.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !meta.file_type().is_socket() {
                    // Don't clobber regular files / dirs / symlinks at
                    // the requested path — that's the user's data.
                    return Err(TransportError::Io(format!(
                        "refusing to bind: {} exists and is not a Unix socket",
                        path.display()
                    )));
                }
                // Probe: a live listener accepts our connect (we
                // immediately drop), a dead one returns
                // `ECONNREFUSED` / `ENOENT` / `ENOTSOCK`. We treat
                // *any* successful connect as "in use", which is
                // strictly safer than the previous behavior of
                // unconditional unlink. The probe is non-blocking
                // because Unix domain sockets don't do a TCP-style
                // 3-way handshake — connect either succeeds or
                // fails immediately.
                match UnixStream::connect(&path) {
                    Ok(s) => {
                        drop(s);
                        return Err(TransportError::Io(format!(
                            "refusing to bind: a peer is already listening at {} \
                             (probed via connect)",
                            path.display()
                        )));
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                        ) =>
                    {
                        // The two error kinds that unambiguously
                        // mean "socket file exists but no listener
                        // is bound" — safe to unlink. Any other
                        // error (permission denied, EINTR, custom
                        // FS errno) is *not* proof of staleness, so
                        // we surface it rather than risk unlinking
                        // a path under a still-live peer.
                        if let Err(e) = std::fs::remove_file(&path) {
                            return Err(TransportError::Io(format!(
                                "stale socket at {} could not be removed: {}",
                                path.display(),
                                e
                            )));
                        }
                    }
                    Err(e) => {
                        return Err(TransportError::Io(format!(
                            "could not probe existing socket at {}: {} \
                             — refusing to unlink without proof it's stale",
                            path.display(),
                            e
                        )));
                    }
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

        let listener = UnixListener::bind(&path)
            .map_err(|e| TransportError::Io(format!("bind {}: {}", path.display(), e)))?;

        // Tighten the socket file's mode to `0600`. The `0700`
        // parent already prevents non-owner traversal, so this is a
        // belt-and-braces hardening for the case where the path is
        // backed up / snapshotted.
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| TransportError::Io(format!("chmod 0600 {}: {}", path.display(), e)))?;

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

    /// Switch the listener into non-blocking mode (Plan 18 C4
    /// follow-up: the CLI accept thread polls + checks a quit flag
    /// so `host.run()` returning can join cleanly without leaking
    /// a thread blocked on `accept()`). Idempotent — safe to call
    /// after construction.
    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<(), TransportError> {
        self.listener
            .set_nonblocking(nonblocking)
            .map_err(|e| TransportError::Io(format!("set_nonblocking: {}", e)))
    }

    /// Try to accept one connection without blocking. Returns
    /// `Ok(None)` when there's no pending connection (the listener
    /// must be in non-blocking mode for this to ever succeed —
    /// callers pair this with [`Self::set_nonblocking`]). `Ok(Some(_))`
    /// for an accepted connection; `Err` for a real I/O failure.
    pub fn try_accept(&self) -> Result<Option<UnixSocketTransport>, TransportError> {
        match self.listener.accept() {
            Ok((stream, _addr)) => Ok(Some(UnixSocketTransport::from_stream(stream)?)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(TransportError::Io(format!("accept: {}", e))),
        }
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
/// perms AND is owned by the current uid.
///
/// Returns `true` when *this call* created the parent directory;
/// the `Drop` impl uses that flag to decide whether the parent is
/// ours to remove. Created via `DirBuilder::mode(0o700)` so the
/// 0700 perms are in place atomically — there is no chmod-after
/// window during which a peer could observe looser perms.
///
/// A pre-existing parent is **validated**, not trusted: the
/// directory's owner uid must match the current process's euid AND
/// the mode must be `& 0o077 == 0` (no group/other bits). This
/// catches the case where a malicious local user planted
/// `/tmp/jian-1000` ahead of time — the mode/uid check refuses to
/// bind inside it. `XDG_RUNTIME_DIR` itself (mode 0700, owner uid)
/// passes naturally.
fn ensure_parent_dir(path: &Path) -> Result<bool, TransportError> {
    let Some(parent) = path.parent() else {
        return Ok(false);
    };
    if parent.as_os_str().is_empty() {
        // Path was a bare filename in the CWD — no parent to make.
        // (The `socket_path::is_legit_unix_path` whitelist refuses
        // bare filenames at the CLI layer, but this branch keeps
        // the function a safe building block for direct callers.)
        return Ok(false);
    }

    // Walk parents bottom-up, materializing each missing component
    // with `mkdir(2)` + mode `0o700` atomically. We do this rather
    // than `create_dir_all` because the latter delegates the chmod
    // to a post-create step, leaving a window where mode is
    // umask-derived. `DirBuilder::mode(0o700).create(p)` is one
    // syscall on Linux/macOS.
    let owner_uid = unix_uid();
    let cleanup_parent = mkdir_chain(parent)?;

    // Final validation — even if we created the parent ourselves,
    // re-stat to confirm `mode == 0o700` & `uid == owner_uid`. A
    // hardened-FS implementation might not honor `mkdir(0o700)`
    // (acl masks, etc.); this catches that and refuses cleanly.
    let meta = std::fs::metadata(parent)
        .map_err(|e| TransportError::Io(format!("stat {}: {}", parent.display(), e)))?;
    if meta.uid() != owner_uid {
        return Err(TransportError::Io(format!(
            "refusing to bind: {} is owned by uid {} (current uid {})",
            parent.display(),
            meta.uid(),
            owner_uid
        )));
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(TransportError::Io(format!(
            "refusing to bind: {} mode 0o{:o} grants group/other access",
            parent.display(),
            mode
        )));
    }

    Ok(cleanup_parent)
}

/// Walk `target`'s ancestors top-down, creating each missing component
/// with `mkdir(p, 0o700)`. Returns whether *this call* created the
/// final (deepest) component — that's the only one we'd ever clean
/// up, because the leaf is the per-product `jian/` subdir; ancestors
/// like `/run/user/1000` are system-managed.
///
/// Implementation detail: we deliberately don't use `create_dir_all`
/// because it doesn't take a mode argument and chmods after creation,
/// leaving a perms-race window. Each `DirBuilder::create` is
/// atomic-via-`mkdir(2)`; an `EEXIST` is benign and means the
/// component already existed.
fn mkdir_chain(target: &Path) -> Result<bool, TransportError> {
    // Collect ancestors top-down (root first → target last).
    let mut chain: Vec<&Path> = target.ancestors().collect();
    chain.reverse();
    let mut created_leaf = false;
    let last = chain.len();
    for (i, dir) in chain.iter().enumerate() {
        if dir.as_os_str().is_empty() {
            continue;
        }
        // Skip the filesystem root — `mkdir("/")` always EEXISTs
        // but the syscall is wasted.
        if dir.parent().is_none() {
            continue;
        }
        match DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {
                if i + 1 == last {
                    created_leaf = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Pre-existing; validation happens after the chain
                // walk (we only validate the *target*, not every
                // ancestor — a system-managed ancestor like
                // `/run/user/1000` may legitimately have looser
                // mode bits than 0700, e.g. some BSDs).
            }
            Err(e) => {
                return Err(TransportError::Io(format!(
                    "mkdir {}: {}",
                    dir.display(),
                    e
                )));
            }
        }
    }
    Ok(created_leaf)
}

#[cfg(unix)]
fn unix_uid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` is an always-succeeding POSIX call.
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_socket_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jian-asp-test-{}-{}", std::process::id(), name));
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
        // Create parent with `0700` so the listener's parent
        // perm-check passes; the case under test is the *socket
        // path itself* being a regular file.
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
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

    // Codex C4 review fix-ups:

    #[test]
    fn bind_refuses_world_readable_parent() {
        // Pre-existing parent dir with mode 0o755 must be refused —
        // a malicious local user could plant such a parent and read
        // anything the listener creates inside.
        let path = temp_socket_path("loose-parent");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = match UnixSocketListener::bind(&path) {
            Ok(_) => panic!("expected refusal"),
            Err(e) => e,
        };
        match err {
            TransportError::Io(msg) => assert!(
                msg.contains("group/other access"),
                "expected mode-bits refusal, got {}",
                msg
            ),
            other => panic!("expected Io, got {:?}", other),
        }
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn bind_refuses_when_live_listener_holds_path() {
        // A still-live UnixListener at the same path should NOT be
        // displaced by a second bind call; the probe must detect it
        // and refuse cleanly. This is the codex HIGH "active
        // listener unlinked as stale" finding.
        let path = temp_socket_path("live-listener");
        let first = UnixSocketListener::bind(&path).expect("first bind");
        // While `first` is live, attempt a second bind on the same
        // path.
        let err = match UnixSocketListener::bind(&path) {
            Ok(_) => panic!("second bind must refuse — first listener is still live"),
            Err(e) => e,
        };
        match err {
            TransportError::Io(msg) => assert!(
                msg.contains("peer is already listening"),
                "expected live-peer refusal, got {}",
                msg
            ),
            other => panic!("expected Io, got {:?}", other),
        }
        // First listener still healthy.
        assert!(path.exists());
        drop(first);
    }

    #[test]
    fn parent_dir_is_atomically_0700_no_chmod_window() {
        // Verifies the fix for the "chmod-after race" finding: when
        // we create the parent fresh, it must be `0o700` from the
        // moment it appears on disk. We can't *prove* atomicity in
        // a unit test, but we can confirm post-bind the dir is
        // exactly 0700 with the *current* umask set to 0o000 (i.e.
        // no implicit tightening) — if the impl relied on
        // umask-aware mkdir, that would surface here.
        let path = temp_socket_path("atomic-perms");
        // SAFETY: `umask(2)` is a thread-unsafe global; this test
        // is single-threaded for the duration so the mutation is
        // contained.
        extern "C" {
            fn umask(mask: u32) -> u32;
        }
        let prev = unsafe { umask(0o000) };
        let listener = UnixSocketListener::bind(&path).expect("bind");
        // Restore umask before any assertion.
        let _ = unsafe { umask(prev) };

        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o700,
            "parent dir must be exactly 0700 even with permissive umask"
        );
        drop(listener);
    }
}
