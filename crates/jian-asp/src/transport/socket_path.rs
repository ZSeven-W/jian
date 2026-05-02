//! Socket path resolution for the local-only ASP transports.
//!
//! Plan 18 ASP prod mode §6 / C4. Two responsibilities:
//!
//! 1. **Resolve `--asp <arg>`** into a concrete bind target.
//!    - `"auto"` derives a per-process default
//!      (`$XDG_RUNTIME_DIR/jian/<pid>.asp.sock` on Unix,
//!      `\\.\pipe\jian\<pid>\asp` on Windows).
//!    - Anything else is treated as an explicit path.
//! 2. **Refuse network bind targets.** Spec §6: "Prod mode must
//!    refuse TCP, `0.0.0.0`, public hostnames, and loopback HTTP
//!    bridges by default." If a future remote-control mode is
//!    required, it is a separate protocol profile with its own
//!    threat model.
//!
//! The resolver is pure: `BindTarget` carries the kind plus the
//! concrete path string; the actual `bind()` lives in the
//! per-platform transport modules.

use std::path::PathBuf;

/// Where the prod ASP server should listen.
///
/// Two variants because the resolver decides at parse time which
/// transport applies, so the per-platform listener doesn't have to
/// re-classify the string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindTarget {
    /// Filesystem path for a Unix domain socket. Always populated on
    /// Unix targets; on Windows this variant is unreachable because
    /// the resolver always picks `NamedPipe` there.
    UnixSocket(PathBuf),
    /// `\\.\pipe\jian\<pid>\asp`-style Named Pipe path. Unreachable
    /// on Unix.
    NamedPipe(String),
}

/// Why a `--asp <arg>` argument was rejected.
#[derive(Debug)]
pub enum BindError {
    /// `arg` looked like a URL (`scheme://...`). Spec §6 forbids any
    /// network transport for prod ASP.
    NetworkBindRefused(String),
    /// `XDG_RUNTIME_DIR` was unset *and* the `/tmp/jian-<uid>`
    /// fallback couldn't be assembled (e.g. `getuid` is mocked-out
    /// in a sandbox).
    AutoPathUnavailable(String),
    /// Caller asked for `auto` on a target that lacks a per-platform
    /// default. Currently unreachable — both unix and windows
    /// branches resolve. Reserved for "wasm32" / future targets.
    AutoUnsupportedOnPlatform(String),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::NetworkBindRefused(s) => write!(
                f,
                "ASP refuses network bind targets — got `{}`. \
                 Use a filesystem path or `auto` (Plan 18 spec §6).",
                s
            ),
            BindError::AutoPathUnavailable(s) => write!(
                f,
                "could not derive an `auto` socket path: {}. \
                 Pass an explicit path with `--asp /tmp/jian.sock`.",
                s
            ),
            BindError::AutoUnsupportedOnPlatform(s) => write!(
                f,
                "`auto` is not supported on this platform: {}. \
                 Pass an explicit path with `--asp <path>`.",
                s
            ),
        }
    }
}

impl std::error::Error for BindError {}

/// Resolve a `--asp` CLI argument into a concrete `BindTarget`.
///
/// Reads `XDG_RUNTIME_DIR` and the calling process's PID via the
/// passed-in helpers so the resolver is unit-testable without
/// touching the real environment. Production callers pass
/// [`std::process::id`] / [`std::env::var_os`].
pub fn resolve_bind_arg(
    arg: &str,
    pid: u32,
    env: impl Fn(&str) -> Option<String>,
) -> Result<BindTarget, BindError> {
    // Reject anything that *looks* like a URL with a scheme. The
    // `://` substring is the simplest, broadest rejection that
    // catches `tcp://`, `http://`, `ws://`, `unix://localhost/...`
    // (some HTTP-bridge libraries use that), and any future scheme
    // a misconfigured deployment might try to thread through. A
    // legitimate filesystem path on every supported platform has no
    // `://` in it.
    if arg.contains("://") {
        return Err(BindError::NetworkBindRefused(arg.to_owned()));
    }

    // Same intent for bare `host:port` strings — `127.0.0.1:9000` /
    // `0.0.0.0:9000`. A real path may contain a `:` (Windows drive
    // letters; macOS HFS legacy `:` separator), but the *combo* of
    // a digit-only segment after the first `:` is a strong network
    // signal and not something a user types by accident.
    if looks_like_host_port(arg) {
        return Err(BindError::NetworkBindRefused(arg.to_owned()));
    }

    if arg == "auto" {
        return resolve_auto(pid, env);
    }

    // Explicit path — defer to platform-specific bind decision.
    #[cfg(unix)]
    {
        Ok(BindTarget::UnixSocket(PathBuf::from(arg)))
    }
    #[cfg(windows)]
    {
        // On Windows an explicit `--asp` argument that starts with
        // `\\.\pipe\` is a Named Pipe; anything else we accept as a
        // pipe name only if it begins with `\\.\pipe\` (the
        // canonical local-pipe namespace). A raw path like
        // `C:\foo.sock` is rejected because Windows pipes don't
        // live on the filesystem; using such a path silently would
        // be the kind of "configured but doesn't bind" footgun the
        // spec calls out.
        if arg.starts_with(r"\\.\pipe\") {
            Ok(BindTarget::NamedPipe(arg.to_owned()))
        } else {
            Err(BindError::NetworkBindRefused(format!(
                "Windows ASP requires a `\\\\.\\pipe\\...` Named Pipe path; got `{}`",
                arg
            )))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(BindError::AutoUnsupportedOnPlatform(format!(
            "no transport for target {}",
            std::env::consts::OS
        )))
    }
}

/// Per-platform `auto` defaults.
fn resolve_auto(pid: u32, env: impl Fn(&str) -> Option<String>) -> Result<BindTarget, BindError> {
    #[cfg(unix)]
    {
        // Preferred: `$XDG_RUNTIME_DIR/jian/<pid>.asp.sock`. The
        // base dir is per-user, mode 0700 by spec on systemd
        // distros, and sticks around for the whole login session.
        if let Some(dir) = env("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                let mut p = PathBuf::from(dir);
                p.push("jian");
                p.push(format!("{}.asp.sock", pid));
                return Ok(BindTarget::UnixSocket(p));
            }
        }
        // Fallback: `/tmp/jian-<uid>/<pid>.asp.sock`. The Unix
        // listener creates the parent dir with 0700 so the socket
        // can't be hijacked by another local user.
        let uid = unix_uid();
        let mut p = PathBuf::from("/tmp");
        p.push(format!("jian-{}", uid));
        p.push(format!("{}.asp.sock", pid));
        Ok(BindTarget::UnixSocket(p))
    }
    #[cfg(windows)]
    {
        // Named Pipes are not filesystem-rooted; the canonical local
        // namespace is `\\.\pipe\<arbitrary>`. Per-pid sub-dir is
        // emulated with a `\jian\<pid>\asp` suffix so two co-running
        // jian processes don't collide on a shared name.
        let _ = env; // not consulted on Windows
        Ok(BindTarget::NamedPipe(format!(
            r"\\.\pipe\jian\{}\asp",
            pid
        )))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, env);
        Err(BindError::AutoUnsupportedOnPlatform(format!(
            "no `auto` default for target {}",
            std::env::consts::OS
        )))
    }
}

/// `host:port` heuristic: at least one `:`, and the segment AFTER
/// the *last* `:` parses as a non-empty all-digit port. This rejects
/// `127.0.0.1:9000` and `[::1]:9000` while letting `C:\path.sock`
/// through (the post-`:` segment isn't all-digits).
fn looks_like_host_port(arg: &str) -> bool {
    let Some(idx) = arg.rfind(':') else {
        return false;
    };
    let port = &arg[idx + 1..];
    !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
}

/// On Unix we need the calling process's UID to assemble the `/tmp`
/// fallback path. We avoid pulling in `libc` here — `geteuid` is a
/// stable POSIX call and the standard library doesn't expose it
/// directly, so we reach through `unsafe extern "C"`.
#[cfg(unix)]
fn unix_uid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` is an always-succeeding POSIX call with no
    // safety preconditions.
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn rejects_tcp_url() {
        let err = resolve_bind_arg("tcp://0.0.0.0:9000", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)), "{}", err);
    }

    #[test]
    fn rejects_http_url() {
        let err = resolve_bind_arg("http://127.0.0.1:8080", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
    }

    #[test]
    fn rejects_ws_url() {
        let err = resolve_bind_arg("ws://localhost:8080", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
    }

    #[test]
    fn rejects_bare_host_port() {
        let err = resolve_bind_arg("127.0.0.1:9000", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
        let err = resolve_bind_arg("0.0.0.0:9000", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
        let err = resolve_bind_arg("localhost:8080", 1234, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
    }

    #[cfg(unix)]
    #[test]
    fn auto_uses_xdg_runtime_dir_when_set() {
        let target = resolve_bind_arg("auto", 4242, |k| {
            if k == "XDG_RUNTIME_DIR" {
                Some("/run/user/1000".into())
            } else {
                None
            }
        })
        .unwrap();
        match target {
            BindTarget::UnixSocket(p) => {
                assert_eq!(p, PathBuf::from("/run/user/1000/jian/4242.asp.sock"));
            }
            other => panic!("expected UnixSocket, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[test]
    fn auto_falls_back_to_tmp_when_xdg_missing() {
        let target = resolve_bind_arg("auto", 4242, empty_env).unwrap();
        match target {
            BindTarget::UnixSocket(p) => {
                let s = p.to_string_lossy();
                assert!(
                    s.starts_with("/tmp/jian-") && s.ends_with("/4242.asp.sock"),
                    "unexpected fallback path `{}`",
                    s
                );
            }
            other => panic!("expected UnixSocket, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[test]
    fn auto_treats_empty_xdg_as_unset() {
        // `XDG_RUNTIME_DIR=""` shouldn't try to bind at the
        // filesystem root. Treat it like unset and fall back.
        let target = resolve_bind_arg("auto", 4242, |k| {
            if k == "XDG_RUNTIME_DIR" {
                Some(String::new())
            } else {
                None
            }
        })
        .unwrap();
        if let BindTarget::UnixSocket(p) = target {
            // String-prefix check — `Path::starts_with` matches whole
            // components, so it would never accept the
            // `/tmp/jian-<uid>` path here.
            let s = p.to_string_lossy();
            assert!(
                s.starts_with("/tmp/jian-") && s.ends_with("/4242.asp.sock"),
                "unexpected fallback path `{}`",
                s
            );
        } else {
            panic!("fallback path expected");
        }
    }

    #[cfg(unix)]
    #[test]
    fn explicit_unix_path_passes_through() {
        let target = resolve_bind_arg("/tmp/foo.sock", 1, empty_env).unwrap();
        assert_eq!(target, BindTarget::UnixSocket(PathBuf::from("/tmp/foo.sock")));
    }

    #[cfg(windows)]
    #[test]
    fn auto_returns_named_pipe_on_windows() {
        let target = resolve_bind_arg("auto", 4242, empty_env).unwrap();
        match target {
            BindTarget::NamedPipe(s) => assert_eq!(s, r"\\.\pipe\jian\4242\asp"),
            other => panic!("expected NamedPipe, got {:?}", other),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_non_pipe_path() {
        let err = resolve_bind_arg(r"C:\foo.sock", 1, empty_env).unwrap_err();
        assert!(matches!(err, BindError::NetworkBindRefused(_)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_accepts_explicit_pipe_path() {
        let target = resolve_bind_arg(r"\\.\pipe\custom\asp", 1, empty_env).unwrap();
        match target {
            BindTarget::NamedPipe(s) => assert_eq!(s, r"\\.\pipe\custom\asp"),
            other => panic!("expected NamedPipe, got {:?}", other),
        }
    }

    #[test]
    fn host_port_heuristic_lets_through_drive_letters() {
        // `C:\foo` is a legit Windows path — no all-digit segment
        // after the trailing `:`. The host:port heuristic must
        // leave it alone (the explicit-path branch then accepts
        // or rejects per platform).
        assert!(!looks_like_host_port(r"C:\foo"));
        assert!(!looks_like_host_port("/tmp/foo.sock"));
        assert!(!looks_like_host_port("auto"));
    }

    #[test]
    fn host_port_heuristic_catches_ipv6_brackets() {
        // `[::1]:9000` ends with an all-digit segment after the
        // last `:`, so the heuristic rejects it as a network bind.
        assert!(looks_like_host_port("[::1]:9000"));
    }
}
