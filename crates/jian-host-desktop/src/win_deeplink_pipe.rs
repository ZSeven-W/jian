//! Windows named-pipe transport for deep-link forwarding (Plan 8
//! §T8 follow-up B).
//!
//! Replaces the round-1 `WM_COPYDATA` + `FindWindowExW` cross-
//! process channel with a named pipe whose DACL is restricted to
//! the calling user's SID. Mirrors the security pattern in
//! `jian_asp::transport::named_pipe`:
//!
//! - **Per-user pipe name**: `\\.\pipe\jian-deeplink-<user_sid>`
//!   so two users on the same Windows host don't collide on the
//!   machine-global pipe namespace. Both primary and secondary
//!   resolve their own SID; in the typical "same-user double
//!   click" path the names match. A different-user secondary
//!   would build a different name and either find no peer or
//!   hit access-denied — exactly the desired behaviour.
//! - **Explicit user-only DACL**: `D:P(A;;GA;;;<user_sid>)` —
//!   `D:P` sets `SE_DACL_PROTECTED` (blocks inherited ACEs); the
//!   single Allow ACE grants `GENERIC_ALL` to the creator's SID
//!   only. No Everyone, no Anonymous, no Admins. This closes the
//!   Plan 8 §T8 round-1 threat-model gap where a same-session
//!   attacker could register a `JianDeepLinkReceiver` window
//!   class and intercept `FindWindowExW`-discovered traffic.
//! - **`PIPE_REJECT_REMOTE_CLIENTS`**: defense in depth —
//!   declines connections via `\\<host>\pipe\…` from off-box
//!   peers.
//! - **`FILE_FLAG_FIRST_PIPE_INSTANCE` + `nMaxInstances = 1`**:
//!   the first `CreateNamedPipeW` succeeds; a second one with the
//!   same name fails with `ERROR_ACCESS_DENIED`. This is the
//!   Windows analogue of refusing a stale Unix socket; combined
//!   with the named-mutex singleton it ensures exactly one
//!   primary listener per user per host.
//!
//! Threading: the listener runs on its own daemon thread so
//! cold-start work on the main thread (file read, schema parse,
//! runtime build, scene walk) doesn't block secondaries' forward.
//! When a URL arrives, the listener `PostMessageW`s the receiver
//! HWND with `WM_USER_DEEPLINK_FORWARD` carrying a heap-leaked
//! `Box<String>` pointer; the receiver `WindowProc` recovers the
//! Box and feeds the URL into `crate::win_deeplink::dispatch_url`.
//! No cross-process state crosses the security boundary except
//! the URL bytes themselves.
//!
//! No graceful shutdown: the listener thread is daemonized and
//! cleans up at process exit. Tests that exercise the bind /
//! forward path do so against unique pipe names so they don't
//! collide with each other or with a developer's running
//! `jian.exe`.

#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, FALSE, HANDLE,
    HWND, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_USER};

/// Custom Windows message the listener thread `PostMessageW`s to
/// the receiver HWND when a URL arrives. The `wParam` is unused
/// (zero); `lParam` carries a heap-leaked `Box<String>` pointer.
/// The receiver `WindowProc` recovers the Box, takes the URL by
/// value, and frees the allocation by dropping the Box.
///
/// `WM_USER + 1` is the canonical first user-defined message ID;
/// pinned via test below so a future addition doesn't accidentally
/// shift this.
pub const WM_USER_DEEPLINK_FORWARD: u32 = WM_USER + 1;

/// Reasonable upper bound on a single URL the listener will read
/// from the pipe. Browsers cap URLs around 2-32 KiB; the Windows
/// `WM_COPYDATA` path used 4 KiB and the same ceiling makes sense
/// here. Combined with `PIPE_BUFFER_BYTES`, a malformed peer can
/// pump at most this much before we reject.
pub const PIPE_URL_MAX_BYTES: usize = 4 * 1024;

/// Buffer size advertised to `CreateNamedPipeW` for the in/out
/// pipe queues. 64 KiB is plenty — the protocol is one URL line
/// per connection — and matches Microsoft's recommendation.
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// Wrap a UTF-8 string as a NUL-terminated UTF-16 wide buffer for
/// the Win32 W APIs. Helper kept private so the conversion stays
/// local to this module.
fn utf16_with_nul(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Resolve the calling process's user SID and return it formatted
/// for SDDL (e.g. `"S-1-5-21-…"`). Mirrors
/// `jian_asp::transport::named_pipe::current_user_sid_string` —
/// both modules build per-user pipe names and per-user DACLs from
/// the same SID, so a single helper would be nicer but keeping it
/// local avoids a `jian-asp` → `jian-host-desktop` dep flip.
///
/// Returns `Err(&'static str)` instead of `TransportError` so this
/// module stays free of `jian-asp` types — keeps the pipe shipping
/// independent of the ASP feature flag.
unsafe fn current_user_sid_string() -> Result<String, &'static str> {
    let mut token: HANDLE = std::ptr::null_mut();
    let proc_h = GetCurrentProcess();
    let opened = OpenProcessToken(proc_h, TOKEN_QUERY, &mut token);
    if opened == FALSE {
        return Err("OpenProcessToken failed");
    }

    // Probe size.
    let mut needed: u32 = 0;
    let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    if needed == 0 {
        let _ = CloseHandle(token);
        return Err("GetTokenInformation(probe) failed");
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
        return Err("GetTokenInformation failed");
    }

    // SAFETY: kernel populated `buf` with `>= sizeof(TOKEN_USER)`
    // bytes. `Vec<u8>::as_ptr()` doesn't guarantee
    // `align_of::<TOKEN_USER>()` — read unaligned to dodge UB.
    let token_user_ptr = buf.as_ptr() as *const TOKEN_USER;
    let token_user = std::ptr::read_unaligned(token_user_ptr);
    let sid = token_user.User.Sid;
    if sid.is_null() {
        return Err("TOKEN_USER.User.Sid was NULL");
    }

    let mut sid_str_ptr: *mut u16 = std::ptr::null_mut();
    let ok = ConvertSidToStringSidW(sid, &mut sid_str_ptr);
    if ok == FALSE || sid_str_ptr.is_null() {
        return Err("ConvertSidToStringSidW failed");
    }

    // Walk the wide string until NUL, copy into a Rust String.
    let mut len = 0usize;
    while *sid_str_ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(sid_str_ptr, len);
    let owned = String::from_utf16_lossy(slice);
    LocalFree(sid_str_ptr as *mut _);
    Ok(owned)
}

/// Build the per-user pipe name. Both primary and secondary derive
/// the same name when they share a SID (the typical case); a
/// cross-user secondary derives a different name and hits "no
/// peer" cleanly. Public so tests + diagnostics can render it.
pub fn user_pipe_name() -> Result<String, &'static str> {
    // SAFETY: bound to the `unsafe fn` body's invariants — see
    // `current_user_sid_string` doc.
    let sid = unsafe { current_user_sid_string()? };
    Ok(format!(r"\\.\pipe\jian-deeplink-{}", sid))
}

/// Outcome of [`forward_url_via_pipe`]. Maps cleanly onto the
/// existing [`crate::win_deeplink_receiver::ForwardOutcome`] so
/// the CLI's branch-and-exit logic doesn't change shape — just
/// the underlying transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeForwardOutcome {
    /// `WriteFile` + `FlushFileBuffers` returned success and the
    /// pipe accepted the URL bytes. Caller should exit success.
    Delivered,
    /// `CreateFileW` returned `INVALID_HANDLE_VALUE` with
    /// `ERROR_FILE_NOT_FOUND` / `ERROR_PIPE_BUSY` — no live
    /// listener at this user's pipe name. Combined with the
    /// `Singleton::Secondary` finding from the named-mutex
    /// probe, this means the primary holds the mutex but its
    /// listener thread isn't running; the CLI should refuse to
    /// start a second window.
    NoPeer,
    /// `CreateFileW` succeeded but `WriteFile` failed (transient
    /// I/O, broken pipe, etc.). Surfaces the Win32 error code.
    SendFailed { last_error: u32 },
}

/// Forward `url` to the running primary's pipe listener.
///
/// Steps:
/// 1. Resolve the calling user's SID, build the pipe name.
/// 2. `CreateFileW(name, GENERIC_WRITE, …)` to open the pipe.
/// 3. `WriteFile(url + '\n')` so the listener's line-reader
///    terminates cleanly.
/// 4. `FlushFileBuffers` so the bytes hit the pipe's kernel
///    buffer before we exit.
/// 5. `CloseHandle`.
///
/// All steps share the same Win32 last-error semantics: failure
/// at any stage maps to `SendFailed { last_error }` except a
/// `NotFound`-class open failure which is `NoPeer`.
pub fn forward_url_via_pipe(url: &str) -> Result<PipeForwardOutcome, &'static str> {
    if url.len() > PIPE_URL_MAX_BYTES {
        return Err("URL exceeds PIPE_URL_MAX_BYTES cap");
    }
    let name = user_pipe_name()?;
    let name_w = utf16_with_nul(&name);

    // Open the pipe with GENERIC_WRITE only — secondaries write,
    // never read. The Read share-mode bit is needed because the
    // primary's pipe is opened with `PIPE_ACCESS_INBOUND` (Read
    // from server's POV; corresponds to Write from our POV).
    // `GENERIC_WRITE` lives at the Foundation seam in
    // windows-sys 0.61.
    use windows_sys::Win32::Foundation::GENERIC_WRITE;
    let handle = unsafe {
        CreateFileW(
            name_w.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let last = unsafe { GetLastError() };
        // ERROR_FILE_NOT_FOUND (2) / ERROR_PIPE_BUSY (231) /
        // ERROR_PIPE_NOT_CONNECTED (233) all collapse to NoPeer
        // — primary's listener isn't accepting. Other errors
        // (access denied, etc.) get the typed variant so the
        // CLI's error message is diagnostic.
        if matches!(last, 2 | 231 | 233) {
            return Ok(PipeForwardOutcome::NoPeer);
        }
        return Ok(PipeForwardOutcome::SendFailed { last_error: last });
    }

    // Write URL + newline. Loop until all bytes flushed (Windows
    // can short-write).
    let mut payload = Vec::with_capacity(url.len() + 1);
    payload.extend_from_slice(url.as_bytes());
    payload.push(b'\n');
    let outcome = write_all(handle, &payload);

    // Flush so the kernel pushes bytes to the pipe before we close.
    if matches!(outcome, PipeForwardOutcome::Delivered) {
        let ok = unsafe { FlushFileBuffers(handle) };
        if ok == FALSE {
            // Best effort — the listener might have already read +
            // disconnected; that's not an error from our POV.
            // Surface only if the kernel reported a real failure.
            let last = unsafe { GetLastError() };
            unsafe {
                let _ = CloseHandle(handle);
            }
            // ERROR_BROKEN_PIPE on flush after a clean read on the
            // peer side is benign.
            if last == ERROR_BROKEN_PIPE {
                return Ok(PipeForwardOutcome::Delivered);
            }
            return Ok(PipeForwardOutcome::SendFailed { last_error: last });
        }
    }

    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(outcome)
}

/// Synchronous `WriteFile` loop. Maps short writes back to the
/// loop and surfaces final-byte failures with the captured
/// `GetLastError`. Returns the typed `PipeForwardOutcome` so
/// callers don't redo the Win32-error mapping.
fn write_all(handle: HANDLE, buf: &[u8]) -> PipeForwardOutcome {
    let mut written_total: usize = 0;
    while written_total < buf.len() {
        let chunk = &buf[written_total..];
        let mut written: u32 = 0;
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
            let last = unsafe { GetLastError() };
            return PipeForwardOutcome::SendFailed { last_error: last };
        }
        if written == 0 {
            return PipeForwardOutcome::SendFailed { last_error: 0 };
        }
        written_total += written as usize;
    }
    PipeForwardOutcome::Delivered
}

/// Wrapper around `HWND` so it can cross thread boundaries. The
/// listener thread reads URLs from the pipe and posts to the HWND;
/// `HWND` is `*mut c_void`, which the windows-sys crate marks
/// `!Send` by default. Posting messages to an HWND from a
/// non-creator thread is allowed (per Microsoft's docs on
/// `PostMessageW`), so the wrapper's `Send` impl is sound.
///
/// We store the handle as `usize` rather than the raw `*mut
/// c_void` alias because Rust's auto-`Send` derivation rejects a
/// closure capturing the wrapper if the inner field is a raw
/// pointer — even with an `unsafe impl Send` on the wrapper, the
/// transitive auto-trait check looks through the field. Storing
/// `usize` keeps the wrapper unambiguously `Send` and the cast
/// back to `HWND` is bit-exact on every Windows target (HWND is
/// pointer-sized).
struct SendableHwnd(usize);

impl SendableHwnd {
    fn new(h: HWND) -> Self {
        Self(h as usize)
    }
    fn get(&self) -> HWND {
        self.0 as HWND
    }
}

/// Spawn the named-pipe listener thread bound to the given
/// receiver `hwnd`. The thread runs as a daemon for the
/// process's lifetime — there's no graceful shutdown path
/// because the typical termination is process exit, where the
/// kernel reclaims pipe handles + the listener thread together.
///
/// Returns `Ok(())` once the pipe is bound (so secondaries
/// arriving immediately after this call find a live listener).
/// On failure the window stays alive but URLs won't route until
/// a follow-up retry succeeds — the function is idempotent;
/// double-call lets the second `CreateNamedPipeW` fail with
/// `ERROR_ACCESS_DENIED` (same name, FIRST_PIPE_INSTANCE) which
/// the caller maps to "already bound, fine".
pub fn install_pipe_listener(hwnd: HWND) -> Result<(), &'static str> {
    let name = user_pipe_name()?;
    let pipe_handle = unsafe { create_listener_pipe(&name)? };

    // Wrap HWND + pipe HANDLE so the closure captures Send-safe
    // wrappers. Inside the thread we cast back to the raw types
    // — those locals don't cross the spawn boundary so the
    // auto-Send check stays happy.
    let hwnd_send = SendableHwnd::new(hwnd);
    let pipe_send = SendableHandle::new(pipe_handle);

    // Daemonize: detached thread, no JoinHandle. Process exit
    // cleans up. Any panic inside the loop is caught with
    // `catch_unwind` so a poisoned URL parse doesn't take down
    // the whole listener.
    std::thread::Builder::new()
        .name("jian-deeplink-pipe-listener".into())
        .spawn(move || {
            let pipe = pipe_send.get();
            let hwnd = hwnd_send.get();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                listener_loop(pipe, hwnd);
            }));
            // SAFETY: handle was created by us; closing on thread
            // exit is the cleanup path.
            unsafe {
                let _ = CloseHandle(pipe);
            }
        })
        .map_err(|_| "failed to spawn deeplink pipe listener thread")?;

    Ok(())
}

/// Wrapper for `HANDLE` to cross thread boundaries. Sibling of
/// `SendableHwnd` — same usize-storage trick to dodge Rust's
/// auto-`Send` rejection of raw-pointer fields.
struct SendableHandle(usize);

impl SendableHandle {
    fn new(h: HANDLE) -> Self {
        Self(h as usize)
    }
    fn get(&self) -> HANDLE {
        self.0 as HANDLE
    }
}

/// Inner listener loop. Runs ConnectNamedPipe → ReadFile (until
/// newline / max bytes) → PostMessageW(WM_USER_DEEPLINK_FORWARD,
/// 0, leaked Box) → DisconnectNamedPipe → repeat.
fn listener_loop(pipe: HANDLE, hwnd: HWND) {
    loop {
        // Block until a peer connects. ConnectNamedPipe returns
        // FALSE both on real errors AND when a peer raced ahead
        // of us between CreateNamedPipe and ConnectNamedPipe (in
        // which case GetLastError is ERROR_PIPE_CONNECTED).
        let ok = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) };
        if ok == FALSE {
            let err = unsafe { GetLastError() };
            if err != ERROR_PIPE_CONNECTED {
                // ERROR_INVALID_HANDLE / pipe closed / similar →
                // exit the loop. Closing happens in the spawn
                // wrapper.
                use std::io::Write;
                let _ = writeln!(
                    std::io::stderr(),
                    "jian-deeplink-pipe-listener: ConnectNamedPipe failed (Win32 {err}); \
                     shutting down listener"
                );
                return;
            }
        }

        // Read a single URL line (terminated by `\n`). The cap
        // keeps a misbehaving peer from forcing an unbounded
        // allocation.
        let url_bytes = read_url_line(pipe);
        // Disconnect whether or not we got valid bytes — the
        // peer's expected to write-and-close, so the disconnect
        // re-arms the pipe for the next forward.
        let _ = unsafe { DisconnectNamedPipe(pipe) };

        let url = match url_bytes {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(_) => continue, // bad utf-8 → drop, listen again
            },
            Err(()) => continue, // truncated / oversized → drop, listen again
        };

        // Heap-leak the URL into a Box so its address survives
        // the PostMessageW return. The receiver's WindowProc
        // recovers via `Box::from_raw` and frees on drop.
        let leaked: *mut String = Box::into_raw(Box::new(url));
        let posted =
            unsafe { PostMessageW(hwnd, WM_USER_DEEPLINK_FORWARD, 0_usize, leaked as isize) };
        if posted == FALSE {
            // HWND invalid (window destroyed?) — recover the Box
            // so we don't leak. Still keep listening; a future
            // primary may rebuild the window via a follow-up.
            unsafe {
                let _ = Box::from_raw(leaked);
            }
            // In practice this means the window is gone and the
            // host is shutting down; exit the listener.
            return;
        }
    }
}

/// Read bytes until newline OR `PIPE_URL_MAX_BYTES`, whichever
/// comes first. Returns the bytes WITHOUT the trailing `\n`.
/// Caps at the max so a peer flooding the pipe can't drive
/// unbounded growth.
///
/// `Err(())` on read errors (broken pipe / etc.) or oversize
/// payload. `Ok(Vec)` even on short read at EOF — the byte
/// stream might have ended with a newline before but no peer
/// closed; treat that as success.
fn read_url_line(pipe: HANDLE) -> Result<Vec<u8>, ()> {
    let mut out = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                pipe,
                byte.as_mut_ptr() as *mut _,
                1,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == FALSE {
            // ERROR_BROKEN_PIPE = 109 → peer closed; if we have
            // bytes, return them. Otherwise empty / error.
            let err = unsafe { GetLastError() };
            if err == ERROR_BROKEN_PIPE && !out.is_empty() {
                return Ok(out);
            }
            return Err(());
        }
        if read == 0 {
            // EOF before newline — accept whatever we have.
            return Ok(out);
        }
        let c = byte[0];
        if c == b'\n' {
            return Ok(out);
        }
        if out.len() >= PIPE_URL_MAX_BYTES {
            return Err(());
        }
        out.push(c);
    }
}

/// Create the listener-side named pipe with a user-only DACL.
/// Mirrors `jian_asp::transport::named_pipe::bind_inner` but
/// listens with `PIPE_ACCESS_INBOUND` (server-reads-only) since
/// secondaries always write the URL and never read.
unsafe fn create_listener_pipe(name: &str) -> Result<HANDLE, &'static str> {
    let user_sid = current_user_sid_string()?;
    let sddl = format!("D:P(A;;GA;;;{})", user_sid);
    let sddl_w = utf16_with_nul(&sddl);

    // Convert SDDL → SECURITY_DESCRIPTOR. The kernel snapshots
    // it during CreateNamedPipeW so we LocalFree it immediately
    // after.
    let mut sd: *mut std::ffi::c_void = std::ptr::null_mut();
    let conv_ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl_w.as_ptr(),
        1, // SDDL_REVISION_1
        &mut sd,
        std::ptr::null_mut(),
    );
    if conv_ok == FALSE {
        return Err("ConvertStringSecurityDescriptorToSecurityDescriptorW failed");
    }
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: FALSE,
    };

    let name_w = utf16_with_nul(name);
    let handle = CreateNamedPipeW(
        name_w.as_ptr(),
        PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
        1, // nMaxInstances — single in-flight forward at a time
        PIPE_BUFFER_BYTES,
        PIPE_BUFFER_BYTES,
        0, // nDefaultTimeOut — unused
        &mut sa,
    );

    // CAPTURE GetLastError BEFORE LocalFree (codex pattern from
    // jian-asp): LocalFree can overwrite thread last-error which
    // would mask the CreateNamedPipeW failure code.
    let create_err = if handle == INVALID_HANDLE_VALUE {
        Some(GetLastError())
    } else {
        None
    };
    LocalFree(sd as *mut _);

    if create_err.is_some() {
        return Err("CreateNamedPipeW failed (already bound? ERROR_ACCESS_DENIED on duplicate)");
    }

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_constants() {
        // Wire compatibility — bumping these requires coordinated
        // primary+secondary updates.
        assert_eq!(WM_USER_DEEPLINK_FORWARD, WM_USER + 1);
        assert_eq!(PIPE_URL_MAX_BYTES, 4 * 1024);
    }

    #[test]
    fn user_pipe_name_starts_with_canonical_prefix() {
        // Don't pin the SID itself (it varies per test runner)
        // but pin the prefix shape so the discovery contract is
        // assertable.
        let name = user_pipe_name().expect("SID should resolve");
        assert!(name.starts_with(r"\\.\pipe\jian-deeplink-"), "got `{name}`");
        // SIDs always start with "S-" (`SECURITY_NULL_SID_RID` etc.).
        let suffix = name.strip_prefix(r"\\.\pipe\jian-deeplink-").unwrap();
        assert!(
            suffix.starts_with("S-"),
            "SID suffix `{suffix}` should start with S-"
        );
    }

    /// On Windows runners, exercise the bind path against a
    /// unique pipe name. Conditional `#[cfg(windows)]` so the
    /// case only fires on a real Windows box.
    #[cfg(windows)]
    #[test]
    fn bind_smoke_test_creates_and_drops_pipe() {
        // Use a unique suffix so concurrent test runners don't
        // race for the same pipe slot. The DACL still restricts
        // to current user; a parallel test in a different user
        // context would just see access-denied and skip.
        let suffix = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        // Bypass `user_pipe_name`'s SID embedding for the test
        // by calling `create_listener_pipe` directly with a
        // unique suffix. This still exercises the DACL build
        // path.
        let name = format!(
            r"\\.\pipe\jian-test-deeplink-{}",
            suffix.replace([':', ' ', '(', ')'], "-")
        );
        let handle = unsafe { create_listener_pipe(&name) }.expect("bind");
        assert_ne!(handle, INVALID_HANDLE_VALUE);
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}
