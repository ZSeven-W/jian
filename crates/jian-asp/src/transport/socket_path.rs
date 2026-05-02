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
    if arg == "auto" {
        return resolve_auto(pid, env);
    }

    // Whitelist the shape of a legitimate explicit path. Anything
    // that *isn't* an absolute / relative-from-CWD filesystem path
    // (Unix) or a `\\.\pipe\` name (Windows) is refused as a
    // possible network bind target — `0.0.0.0`, `localhost`, `999`,
    // `tcp:9000`, `[::1]:80`, `127.0.0.1` (no port), and any future
    // scheme a misconfigured deployment might thread through.
    //
    // The whitelist is broader than the previous heuristic blacklist
    // because the threat-model wants any *uncertain* shape closed by
    // default; a user who legitimately wants to bind a relative
    // socket path can write `./my.sock` or `../shared/my.sock`.
    #[cfg(unix)]
    {
        if !is_legit_unix_path(arg) {
            return Err(BindError::NetworkBindRefused(arg.to_owned()));
        }
        Ok(BindTarget::UnixSocket(PathBuf::from(arg)))
    }
    #[cfg(windows)]
    {
        // Windows pipes don't live on the filesystem; the canonical
        // local-pipe namespace starts with `\\.\pipe\`. A raw path
        // like `C:\foo.sock` is rejected because using it would be
        // the kind of "configured but doesn't bind" footgun the spec
        // calls out.
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
        let _ = (pid, env);
        Err(BindError::AutoUnsupportedOnPlatform(format!(
            "no transport for target {}",
            std::env::consts::OS
        )))
    }
}

/// Whitelist for explicit Unix `--asp <arg>` values.
///
/// Accept exactly two shapes:
/// - **Absolute path**: starts with `/`. `/tmp/foo.sock`, `/run/...`.
/// - **Relative-from-CWD path**: starts with `./` or `../`.
///   The leading `./` is mandatory so a bare `foo` (which could be
///   a hostname) is refused; a user who really wants the CWD writes
///   `./foo.sock`.
///
/// Both shapes additionally cannot contain `://` (catches
/// `unix:///tmp/x` and other URI variants — strip the scheme
/// yourself if you want a plain path).
#[cfg(unix)]
fn is_legit_unix_path(arg: &str) -> bool {
    if arg.contains("://") {
        return false;
    }
    arg.starts_with('/') || arg.starts_with("./") || arg.starts_with("../")
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
        Ok(BindTarget::NamedPipe(format!(r"\\.\pipe\jian\{}\asp", pid)))
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

    // Codex review found that the previous heuristic let through
    // bare-host / port-only / `tcp:` strings. The new whitelist
    // requires a path-shape prefix; pin every false-negative case
    // explicitly so a future blacklist refactor would trip here.
    #[cfg(unix)]
    #[test]
    fn rejects_bare_ip_no_port() {
        for s in ["0.0.0.0", "127.0.0.1", "::1", "[::]"] {
            let err = resolve_bind_arg(s, 1234, empty_env).unwrap_err();
            assert!(
                matches!(err, BindError::NetworkBindRefused(_)),
                "expected refusal for {}, got {:?}",
                s,
                err
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_hostname_only() {
        for s in ["localhost", "example.com", "my-server"] {
            let err = resolve_bind_arg(s, 1234, empty_env).unwrap_err();
            assert!(
                matches!(err, BindError::NetworkBindRefused(_)),
                "expected refusal for {}",
                s
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_port_only() {
        for s in ["999", "9000", "0"] {
            let err = resolve_bind_arg(s, 1234, empty_env).unwrap_err();
            assert!(
                matches!(err, BindError::NetworkBindRefused(_)),
                "expected refusal for {}",
                s
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_tcp_scheme_alias() {
        // `tcp:9000`, `unix:foo`, `udp:9000` — `:` without `//`.
        for s in ["tcp:9000", "unix:foo", "udp:9000", "ipc:my.sock"] {
            let err = resolve_bind_arg(s, 1234, empty_env).unwrap_err();
            assert!(
                matches!(err, BindError::NetworkBindRefused(_)),
                "expected refusal for {}",
                s
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_bare_relative_filename() {
        // `foo` could be a hostname; require `./foo` to disambiguate.
        for s in ["foo", "foo.sock", "my.sock"] {
            let err = resolve_bind_arg(s, 1234, empty_env).unwrap_err();
            assert!(
                matches!(err, BindError::NetworkBindRefused(_)),
                "expected refusal for {}",
                s
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn accepts_explicit_relative_path() {
        let target = resolve_bind_arg("./foo.sock", 1, empty_env).unwrap();
        assert_eq!(target, BindTarget::UnixSocket(PathBuf::from("./foo.sock")));
        let target = resolve_bind_arg("../up/foo.sock", 1, empty_env).unwrap();
        assert_eq!(
            target,
            BindTarget::UnixSocket(PathBuf::from("../up/foo.sock"))
        );
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
        assert_eq!(
            target,
            BindTarget::UnixSocket(PathBuf::from("/tmp/foo.sock"))
        );
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
}
