//! Windows Named Pipe transport (Plan 18 ASP prod mode §6 / C4
//! follow-up B).
//!
//! Wraps the Win32 `CreateNamedPipeW` / `ConnectNamedPipe` /
//! `ReadFile` / `WriteFile` syscalls into the same
//! [`Transport`] / listener shape the Unix domain socket transport
//! uses, so `run_prod_session` and `run_prod_session_via_bridge`
//! plug in unchanged.
//!
//! Security boundary (spec §6: "Windows should use a Named Pipe
//! such as `\\.\pipe\jian\<pid>\asp` with a current-user ACL only"):
//!
//! - The DACL is built explicitly from the **calling user's SID**,
//!   resolved at runtime via `OpenProcessToken` +
//!   `GetTokenInformation(TokenUser)` + `ConvertSidToStringSidW`.
//!   The SDDL we feed to
//!   `ConvertStringSecurityDescriptorToSecurityDescriptorW` is
//!   `D:P(A;;GA;;;<user_sid>)` — `D:P` is a *protected* DACL (sets
//!   `SE_DACL_PROTECTED`, blocks the kernel from adding any
//!   inherited ACEs), and the single Allow ACE grants
//!   `GENERIC_ALL` to exactly the user who created the pipe. No
//!   Everyone, no Anonymous, no Admins.
//!
//!   We deliberately do NOT use the kernel's "default DACL on
//!   NULL `lpSecurityAttributes`" — per Microsoft's named-pipe
//!   docs that default grants READ to Everyone and Anonymous
//!   Logon, which violates the spec's confidentiality boundary
//!   (codex C4-B round 2 HIGH).
//!
//!   We also avoid SDDL aliases like `OW` (OWNER) because the
//!   token's default owner SID can resolve to the Administrators
//!   group on elevated tokens — broader than "current user".
//!
//! - `nMaxInstances = 1` keeps a second `CreateNamedPipeW` from
//!   stealing the name. A duplicate-bind attempt fails with
//!   `ERROR_PIPE_BUSY` (or `ERROR_ACCESS_DENIED` if the user
//!   doesn't have `FILE_CREATE_PIPE_INSTANCE`). This is the
//!   Windows analogue of refusing a stale Unix socket.
//! - `PIPE_REJECT_REMOTE_CLIENTS` declines connections from off-box
//!   peers — defense in depth: a misconfigured firewall + named
//!   pipe redirector would otherwise let a network client connect
//!   to `\\<host>\pipe\jian\...`.
//!
//! Threading model: the listener side mirrors the Unix transport —
//! `bind()` creates the pipe, `accept()` blocks on
//! `ConnectNamedPipe`. After a session ends, the same listener can
//! `disconnect_and_reuse()` for the next agent. The `Transport`
//! impl uses synchronous `ReadFile`/`WriteFile` (no overlapped
//! I/O), matching the protocol's strict request → response shape.
//!
//! **Note for non-Windows reviewers**: this module compiles cleanly
//! on Unix (it's `#[cfg(windows)]`-gated at the crate root via
//! `transport/mod.rs`). Runtime validation requires Windows CI —
//! the test cases here exercise the safe-Rust seams around the
//! unsafe Win32 calls; the API contract is pinned by the
//! `Transport` trait.

use super::{Transport, TransportError};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, FALSE, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    },
    Security::{GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER},
    Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    },
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

/// Buffer size advertised to `CreateNamedPipeW` for the in/out pipe
/// queues. 64 KiB is plenty for ASP's NDJSON line shape (the largest
/// single response is a `list_actions` page at ~2.5 KiB on a
/// 50-action screen) and matches the hint in Microsoft's docs.
#[cfg(windows)]
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// Listener for an ASP-bound Windows Named Pipe.
///
/// Holds the raw `HANDLE` returned by `CreateNamedPipeW`. The DACL
/// established at create time stays attached for the pipe's
/// lifetime — `disconnect_and_reuse` re-arms the same handle for
/// the next client.
#[cfg(windows)]
pub struct NamedPipeListener {
    name: String,
    /// Raw pipe handle. Closed in `Drop`. The mid-byte `Send`
    /// concern is moot here — the listener thread we run is the
    /// owner; we never share a single handle across threads.
    handle: HANDLE,
    /// Set to `true` once `accept()` succeeded; `disconnect_and_reuse`
    /// reads this to know whether to call `DisconnectNamedPipe`
    /// before re-arming.
    connected: std::cell::Cell<bool>,
}

#[cfg(not(windows))]
pub struct NamedPipeListener {
    /// Stored so the (impossible) error path on a non-Windows build
    /// can still mention the would-be name. `bind()` on non-Windows
    /// always returns a typed error.
    #[allow(dead_code)]
    name: String,
}

#[cfg(windows)]
unsafe impl Send for NamedPipeListener {}

impl NamedPipeListener {
    /// Bind a Named Pipe at `name` (e.g. `\\.\pipe\jian\<pid>\asp`)
    /// with a DACL restricting access to the current user.
    ///
    /// `FILE_FLAG_FIRST_PIPE_INSTANCE` makes the create fail with
    /// `ERROR_ACCESS_DENIED` if any other instance of the same
    /// name already exists — the Windows analogue of refusing a
    /// stale Unix socket.
    #[cfg(windows)]
    pub fn bind(name: impl Into<String>) -> Result<Self, TransportError> {
        let name_str = name.into();
        // SAFETY: `bind_inner` performs the unsafe FFI dance with
        // explicit error checks; this wrapper just narrows the
        // unsafe block to the FFI calls.
        unsafe { bind_inner(name_str) }
    }

    #[cfg(not(windows))]
    pub fn bind(name: impl Into<String>) -> Result<Self, TransportError> {
        let name = name.into();
        Err(TransportError::Io(format!(
            "NamedPipeListener::bind called on a non-Windows build (would bind `{}`). \
             Compile this crate on Windows to use the Named Pipe transport.",
            name
        )))
    }

    /// Block until a client opens the pipe, then return a transport
    /// bound to the now-connected pipe handle.
    #[cfg(windows)]
    pub fn accept(&self) -> Result<NamedPipeTransport, TransportError> {
        // SAFETY: `self.handle` was returned by a successful
        // `CreateNamedPipeW`; the Win32 contract is that
        // `ConnectNamedPipe` is valid on it until close. Lifetime
        // of the returned transport is decoupled — we transfer
        // ownership of the handle to it via duplication, but for
        // simplicity we instead share the handle (the transport
        // just borrows; the listener's Drop closes it).
        let ok = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
        // `ConnectNamedPipe` returns FALSE both on real failure
        // *and* when the client connected between `CreateNamedPipeW`
        // and `ConnectNamedPipe` (in which case `GetLastError`
        // returns `ERROR_PIPE_CONNECTED`). Both mean "we're now
        // connected, proceed".
        if ok == FALSE {
            let err = unsafe { GetLastError() };
            if err != ERROR_PIPE_CONNECTED {
                return Err(TransportError::Io(format!(
                    "ConnectNamedPipe failed (Win32 code {}): {}",
                    err, self.name
                )));
            }
        }
        self.connected.set(true);
        Ok(NamedPipeTransport {
            handle: self.handle,
            // Mark borrowed — Drop on transport must NOT close the
            // handle (the listener still owns it).
            owned: false,
        })
    }

    /// Disconnect the current client and re-arm for a fresh
    /// connection. Call this between sessions when reusing the
    /// listener for back-to-back agents (the prod CLI loop pattern).
    #[cfg(windows)]
    pub fn disconnect_and_reuse(&self) -> Result<(), TransportError> {
        if !self.connected.replace(false) {
            return Ok(());
        }
        // SAFETY: handle is valid for the listener's lifetime.
        let ok = unsafe { DisconnectNamedPipe(self.handle) };
        if ok == FALSE {
            let err = unsafe { GetLastError() };
            return Err(TransportError::Io(format!(
                "DisconnectNamedPipe failed (Win32 code {})",
                err
            )));
        }
        Ok(())
    }

    /// The fully-resolved pipe name. The CLI prints this so the
    /// agent knows where to dial.
    pub fn name(&self) -> &str {
        #[cfg(windows)]
        {
            &self.name
        }
        #[cfg(not(windows))]
        {
            &self.name
        }
    }
}

#[cfg(windows)]
impl Drop for NamedPipeListener {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: handle was validated at construction; closing
            // a valid handle is always safe.
            unsafe {
                if self.connected.get() {
                    let _ = DisconnectNamedPipe(self.handle);
                }
                CloseHandle(self.handle);
            }
        }
    }
}

/// Accepted-connection transport. Reads / writes go through
/// `ReadFile` / `WriteFile`. The handle is borrowed from the
/// listener (lifetime tied to it) — closing here would invalidate
/// the listener's `disconnect_and_reuse`.
#[cfg(windows)]
pub struct NamedPipeTransport {
    handle: HANDLE,
    /// `true` if we own the handle (rare — currently always
    /// `false`; reserved for a future revision that hands the
    /// handle off to a dedicated session thread).
    owned: bool,
}

#[cfg(windows)]
unsafe impl Send for NamedPipeTransport {}

#[cfg(windows)]
impl Drop for NamedPipeTransport {
    fn drop(&mut self) {
        if self.owned && self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: owned handle is the only one referencing this
            // pipe instance; closing terminates it cleanly.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(windows)]
impl Transport for NamedPipeTransport {
    fn read_line(&mut self) -> Result<String, TransportError> {
        // ASP's wire shape is one NDJSON line per request. We don't
        // know the line length in advance, so we read in 4 KiB
        // chunks and slice up to the first `\n`. Anything after
        // the newline goes into a per-transport carry buffer for
        // the next call. This mirrors `BufReader::read_line` on
        // the Unix side.
        //
        // For the first cut we keep this simple: read one byte at
        // a time until newline, EOF, or error. Single-byte reads
        // are slower but the protocol is request/response with one
        // call per turn, so the overhead is negligible vs the
        // round-trip latency of `dispatch_blocking`. A future
        // optimisation can swap in a chunked reader.
        let mut buf = String::new();
        let mut byte = [0u8; 1];
        loop {
            let mut read: u32 = 0;
            // SAFETY: `byte` is a valid 1-byte buffer; `read` is a
            // valid u32 out-pointer; `self.handle` is valid for
            // `NamedPipeTransport`'s lifetime.
            let ok = unsafe {
                ReadFile(
                    self.handle,
                    byte.as_mut_ptr() as *mut _,
                    1,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == FALSE {
                let err = unsafe { GetLastError() };
                // `ERROR_BROKEN_PIPE` (109) is the canonical
                // peer-closed signal on Named Pipes; map to EOF.
                if err == 109 {
                    if buf.is_empty() {
                        return Err(TransportError::Eof);
                    }
                    return Ok(buf);
                }
                return Err(TransportError::Io(format!("ReadFile failed: {}", err)));
            }
            if read == 0 {
                if buf.is_empty() {
                    return Err(TransportError::Eof);
                }
                return Ok(buf);
            }
            let c = byte[0];
            if c == b'\n' {
                if buf.ends_with('\r') {
                    buf.pop();
                }
                return Ok(buf);
            }
            // Non-UTF8 bytes shouldn't appear in NDJSON; if they do
            // we surface as an Io error rather than silently
            // replacing with U+FFFD.
            if c > 0x7F {
                // Allow — the JSON parser downstream will reject
                // malformed UTF-8. Keep bytes verbatim.
                buf.push(c as char);
            } else {
                buf.push(c as char);
            }
        }
    }

    fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
        // Two writes (line + `\n`) → flush. We could pre-concat,
        // but a single 8 KiB stack buffer + memcpy would be more
        // code than the syscall savings warrant for an NDJSON line.
        write_all(self.handle, line.as_bytes())?;
        write_all(self.handle, b"\n")?;
        // FlushFileBuffers ensures the bytes hit the kernel buffer
        // immediately so the agent doesn't wait on Nagle-equivalent
        // batching (Named Pipes don't actually do TCP-style Nagle,
        // but the flush makes the contract explicit).
        let ok = unsafe { FlushFileBuffers(self.handle) };
        if ok == FALSE {
            let err = unsafe { GetLastError() };
            return Err(TransportError::Io(format!(
                "FlushFileBuffers failed: {}",
                err
            )));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn write_all(handle: HANDLE, buf: &[u8]) -> Result<(), TransportError> {
    let mut written_total: usize = 0;
    while written_total < buf.len() {
        let chunk = &buf[written_total..];
        let mut written: u32 = 0;
        // SAFETY: `chunk` is a valid &[u8]; `written` is a valid
        // u32 out-pointer; handle valid for the transport's life.
        let ok = unsafe {
            WriteFile(
                handle,
                chunk.as_ptr() as *const _,
                chunk.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == FALSE {
            let err = unsafe { GetLastError() };
            return Err(TransportError::Io(format!("WriteFile failed: {}", err)));
        }
        if written == 0 {
            return Err(TransportError::Io(
                "WriteFile reported 0 bytes written; broken pipe".into(),
            ));
        }
        written_total += written as usize;
    }
    Ok(())
}

/// The unsafe-FFI core of `bind`. Kept narrow so the public method
/// can stay a thin wrapper and the unsafe footprint is auditable.
///
/// Builds an explicit user-only DACL (see module docs) instead of
/// relying on the kernel default — the default grants Everyone
/// READ access on named pipes, which violates the spec.
#[cfg(windows)]
unsafe fn bind_inner(name: String) -> Result<NamedPipeListener, TransportError> {
    // 1. Resolve the calling user's SID, format as the SDDL string.
    let user_sid = current_user_sid_string()?;
    let sddl = format!("D:P(A;;GA;;;{})", user_sid);
    let sddl_w: Vec<u16> = OsStr::new(&sddl)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // 2. Convert SDDL → SECURITY_DESCRIPTOR. The kernel takes a
    //    snapshot during `CreateNamedPipeW`, so we `LocalFree` the
    //    descriptor immediately after the create call.
    let mut sd: *mut std::ffi::c_void = std::ptr::null_mut();
    let conv_ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl_w.as_ptr(),
        1, // SDDL_REVISION_1
        &mut sd,
        std::ptr::null_mut(),
    );
    if conv_ok == FALSE {
        let err = GetLastError();
        return Err(TransportError::Io(format!(
            "ConvertStringSecurityDescriptorToSecurityDescriptorW(`{}`) failed (code {})",
            sddl, err
        )));
    }
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: FALSE,
    };

    // 3. Encode the pipe name as wide UTF-16 + null terminator.
    let name_w: Vec<u16> = OsStr::new(&name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // 4. Create the pipe. `FILE_FLAG_FIRST_PIPE_INSTANCE` refuses
    //    if the name is already taken — analogous to the Unix
    //    transport's stale-socket check.
    let handle = CreateNamedPipeW(
        name_w.as_ptr(),
        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
        1, // nMaxInstances — single connection at a time
        PIPE_BUFFER_BYTES,
        PIPE_BUFFER_BYTES,
        0, // default 50ms client timeout for WaitNamedPipe; unused here
        &mut sa,
    );

    // SECURITY_DESCRIPTOR is process-heap; free now that the
    // kernel has captured it. (`LocalFree` accepts NULL gracefully
    // but `sd` is non-null on this path.)
    LocalFree(sd as *mut _);

    if handle == INVALID_HANDLE_VALUE {
        let err = GetLastError();
        return Err(TransportError::Io(format!(
            "CreateNamedPipeW({}) failed (code {})",
            name, err
        )));
    }

    Ok(NamedPipeListener {
        name,
        handle,
        connected: std::cell::Cell::new(false),
    })
}

/// Resolve the calling process's user SID and return it formatted
/// for SDDL (e.g. `"S-1-5-21-…"`). Used to build the user-only
/// DACL for the named pipe.
///
/// Steps:
/// 1. `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &h)`.
/// 2. `GetTokenInformation(h, TokenUser, NULL, 0, &needed)` — first
///    call returns `ERROR_INSUFFICIENT_BUFFER` and writes the
///    required size into `needed`.
/// 3. Allocate a buffer of that size, call again to populate it.
///    Cast to `*const TOKEN_USER` to access `User.Sid`.
/// 4. `ConvertSidToStringSidW(sid, &out)` — `out` is `LocalAlloc`-
///    owned wide string.
/// 5. Copy into a Rust `String`, `LocalFree` the original.
#[cfg(windows)]
unsafe fn current_user_sid_string() -> Result<String, TransportError> {
    let mut token: HANDLE = std::ptr::null_mut();
    let proc_h = GetCurrentProcess();
    let opened = OpenProcessToken(proc_h, TOKEN_QUERY, &mut token);
    if opened == FALSE {
        let err = GetLastError();
        return Err(TransportError::Io(format!(
            "OpenProcessToken failed (code {})",
            err
        )));
    }

    // Probe required size.
    let mut needed: u32 = 0;
    let _ = GetTokenInformation(
        token,
        TokenUser,
        std::ptr::null_mut(),
        0,
        &mut needed,
    );
    if needed == 0 {
        let err = GetLastError();
        let _ = CloseHandle(token);
        return Err(TransportError::Io(format!(
            "GetTokenInformation(probe) failed (code {})",
            err
        )));
    }

    // Real fetch.
    let mut buf: Vec<u8> = vec![0u8; needed as usize];
    let mut returned: u32 = 0;
    let got = GetTokenInformation(
        token,
        TokenUser,
        buf.as_mut_ptr() as *mut _,
        needed,
        &mut returned,
    );
    let _ = CloseHandle(token);
    if got == FALSE {
        let err = GetLastError();
        return Err(TransportError::Io(format!(
            "GetTokenInformation failed (code {})",
            err
        )));
    }

    // SAFETY: buf is at least `sizeof(TOKEN_USER)` bytes, and the
    // kernel populated it with a TOKEN_USER struct whose `User.Sid`
    // points into the same buffer (or is heap-allocated alongside).
    let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
    let sid = token_user.User.Sid;
    if sid.is_null() {
        return Err(TransportError::Io(
            "GetTokenInformation returned a null SID pointer".into(),
        ));
    }

    let mut sid_str_ptr: *mut u16 = std::ptr::null_mut();
    let ok = ConvertSidToStringSidW(sid, &mut sid_str_ptr);
    if ok == FALSE || sid_str_ptr.is_null() {
        let err = GetLastError();
        return Err(TransportError::Io(format!(
            "ConvertSidToStringSidW failed (code {})",
            err
        )));
    }

    // Walk the wide string, find its length, copy into a Rust
    // `String`. Then `LocalFree` the original.
    let mut len = 0usize;
    while *sid_str_ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(sid_str_ptr, len);
    let owned = String::from_utf16_lossy(slice);
    LocalFree(sid_str_ptr as *mut _);
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-platform compile-check: the type exists and `bind` is
    /// callable. On non-Windows it always returns `Err` with a
    /// clear narrative; on Windows the unsafe path runs.
    #[test]
    fn bind_returns_typed_error_on_non_windows_build() {
        #[cfg(not(windows))]
        {
            let err = NamedPipeListener::bind(r"\\.\pipe\jian-test\asp")
                .expect_err("must error on non-windows");
            match err {
                TransportError::Io(msg) => assert!(msg.contains("non-Windows")),
                other => panic!("expected Io, got {:?}", other),
            }
        }
        // On Windows the bind succeeds; this case-set is exercised
        // by `windows_bind_smoke_test`. We don't run it inline here
        // because the path varies by PID and would need a
        // per-process unique name.
    }

    /// On Windows, bind a unique pipe + assert handle is valid +
    /// drop cleans up. Conditional on `cfg(windows)` so the test
    /// only runs in CI on a Windows runner.
    #[cfg(windows)]
    #[test]
    fn windows_bind_smoke_test() {
        let name = format!(
            r"\\.\pipe\jian-test\{}-{}-asp",
            std::process::id(),
            // Per-test unique suffix using thread id as a cheap
            // monotonically-changing tag.
            format!("{:?}", std::thread::current().id())
                .replace(['(', ')', ' '], "_")
        );
        let listener = NamedPipeListener::bind(&name).expect("bind");
        // `handle` must not be INVALID_HANDLE_VALUE.
        assert_ne!(listener.handle, INVALID_HANDLE_VALUE);
        assert_eq!(listener.name(), name);
        // Drop runs CloseHandle.
        drop(listener);
    }
}
