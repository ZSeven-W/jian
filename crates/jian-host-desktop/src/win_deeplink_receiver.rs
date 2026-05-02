//! Windows hidden message-only window + named-mutex single-instance
//! plumbing (Plan 8 §T8). Pairs with macOS's
//! [`crate::apple_event_receiver`].
//!
//! ## Flow
//!
//! 1. **Primary instance**: at startup, calls
//!    [`try_acquire_singleton`]. The named `CreateMutexW` succeeds
//!    with no `ERROR_ALREADY_EXISTS`, so we own the mutex for the
//!    process lifetime. Then [`install_receiver_window`] registers a
//!    `WNDCLASSEXW` named [`crate::win_deeplink::RECEIVER_CLASS_NAME`],
//!    creates a `HWND_MESSAGE` window, and installs a custom
//!    `WindowProc` that decodes incoming `WM_COPYDATA` messages and
//!    forwards them into [`crate::win_deeplink::dispatch_url`].
//! 2. **Secondary instance** (same `jian.exe`, second click on a
//!    `jian://...` URL or `.op` file): [`try_acquire_singleton`]
//!    returns [`Singleton::Secondary`]. The CLI then calls
//!    [`forward_url_to_primary`], which `FindWindowExW(HWND_MESSAGE,
//!    NULL, JianDeepLinkReceiver, NULL)` to locate the primary's
//!    receiver window, packs the URL string into a `COPYDATASTRUCT`,
//!    and `SendMessageW(peer, WM_COPYDATA, 0, &cds)`. The CLI then
//!    exits with success — the URL was delivered to the running app.
//! 3. The primary's `WindowProc` validates the `COPYDATASTRUCT`'s
//!    `dwData` tag, copies the bytes (the secondary's address space
//!    is gone the moment `SendMessageW` returns and the kernel-side
//!    cross-process copy buffer is read-only inside the receiver),
//!    decodes UTF-16 LE → `String`, and calls
//!    `win_deeplink::dispatch_url`.
//!
//! ## Why named mutex (not `FindWindowW`-only)
//!
//! Two startup races would otherwise let a secondary instance run
//! its full event loop:
//! - The first `jian.exe` is mid-startup and hasn't yet created the
//!   receiver window when the second instance fires up. `FindWindowW`
//!   returns NULL → secondary falsely thinks it's primary.
//! - The first `jian.exe` is exiting and has destroyed the window
//!   but its process is still alive cleaning up. Same false-primary.
//!
//! `CreateMutexW(name)` is atomic at the kernel level: any second
//! caller observes `ERROR_ALREADY_EXISTS` even before the first
//! caller's window exists. The mutex is held for the lifetime of
//! [`SingletonGuard`] and released on Drop.
//!
//! ## Validation
//!
//! Runtime validation needs a real Windows host: `cargo test
//! --target x86_64-pc-windows-msvc` will compile-check the FFI
//! surface, but `WM_COPYDATA` cross-process delivery only exercises
//! against a live message pump. Unit tests below pin the wire-shape
//! constants (the `dwData` tag, the singleton-name shape) so a
//! drift across primary/secondary versions surfaces as a test
//! failure rather than a silent message drop.
//!
//! ## Known gap: cold-start message-pump latency (codex round 4 MEDIUM)
//!
//! `SetEvent` on the ready event fires when `install_receiver_window`
//! returns — i.e. as soon as the HWND is created. The main thread
//! then continues with the host's startup work (file read, schema
//! parse, runtime build, scene walk) before reaching `host.run()`
//! and entering winit's message pump. During that 30-100 ms gap a
//! secondary's `SendMessageTimeoutW` lands in the kernel queue and
//! waits for the main thread to dispatch. With the 5-second
//! timeout this is benign for typical cold-start, but a slow disk
//! / huge `.op` could push past the budget. The clean fix is a
//! dedicated message-pump thread for the receiver window
//! (independent of winit's main thread); that lands in a follow-up
//! since it touches `DesktopHost` plumbing.
//!
//! ## Threat-model gap (codex round 1 MEDIUM Q4)
//!
//! `FindWindowExW` authenticates the peer only by class-name
//! string. A same-session, same-or-lower-integrity attacker can
//! register a `JianDeepLinkReceiver` window before our primary
//! and intercept inbound URLs. The `dwData == COPYDATA_TAG` check
//! defends the inbound side (the real receiver ignores spoof
//! traffic) but doesn't help the secondary — its `FindWindowExW`
//! result IS the attacker. Mitigations belong in a follow-up
//! that switches to a named pipe with an explicit user-SID DACL
//! (mirror of the `jian-asp` Windows transport's pattern),
//! authenticating the peer by SID rather than window-class
//! string. Same-user attackers already have broad write access
//! to user-owned resources, so this gap is defense-in-depth, not
//! load-bearing — but the named-pipe migration is on the
//! Plan 8 §T8 follow-up list.

#![cfg(target_os = "windows")]

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

thread_local! {
    /// Holds the receiver-window guard for the running primary so
    /// its `Drop` doesn't fire prematurely. Mirrors the pattern in
    /// `apple_event_receiver` — the FFI-registered window must
    /// outlive the call that creates it. `RefCell` lets the
    /// thread-local replace the value when the host shuts down.
    static RECEIVER_WINDOW: RefCell<Option<ReceiverWindow>> =
        const { RefCell::new(None) };
}
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_CLASS_ALREADY_EXISTS, ERROR_TIMEOUT,
    HANDLE, HMODULE, HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject,
};
// `SYNCHRONIZE` lives under Storage::FileSystem in windows-sys 0.61
// (the access-rights bitmask is shared across kernel objects but the
// constant ended up in that module). Re-import here so the OpenEventW
// access-mask parameter resolves cleanly.
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, RegisterClassExW,
    SendMessageTimeoutW, CW_USEDEFAULT, HMENU, HWND_MESSAGE, SMTO_BLOCK, WM_COPYDATA, WNDCLASSEXW,
    WS_OVERLAPPED,
};

/// Tag carried by every `WM_COPYDATA` message we send between
/// primary / secondary instances. A receiver that sees a different
/// tag (some other app spamming WM_COPYDATA) silently ignores the
/// payload — Windows's `FindWindowExW` filter on class name should
/// already keep cross-app traffic out, but tag-checking is the
/// defense-in-depth that a class-name spoofer can't bypass.
///
/// Pinned by a unit test below; bumping the value requires
/// coordinated primary+secondary updates in the same release.
pub const COPYDATA_TAG: usize = 0x4A_44_4C_31; // 'JDL1'

/// Maximum `cbData` we'll deserialise. Codex round 2 MEDIUM: a
/// same-session sender knowing the public tag could otherwise
/// force an arbitrarily large allocation in `from_raw_parts` /
/// `String::from_utf16`. URLs in the wild top out at ~2 KiB
/// (RFC 7230 §3.1.1 leaves it implementation-defined; browsers
/// cap around 2-32 KiB). 4 KiB is a generous ceiling that still
/// fits two pages of memory, well below anything that could
/// pressure the runtime.
pub const COPYDATA_MAX_BYTES: usize = 4 * 1024;

/// Named mutex used to detect "another `jian.exe` is already
/// running" within the current Windows session. Codex round 2
/// MEDIUM: `Global\` prefix would scope across sessions, but
/// `HWND_MESSAGE` window discovery (`FindWindowExW`) is itself
/// session-scoped — a cross-session match would convince this
/// process that a peer exists while leaving us unable to forward.
/// All three synchronisation primitives (mutex, ready event,
/// receiver window) MUST be in the same session, so we use the
/// `Local\` namespace.
pub const SINGLETON_MUTEX_NAME: &str = r"Local\JianHostDesktop-Singleton";

/// Outcome of [`try_acquire_singleton`]. The CLI branches on this
/// at startup:
///
/// - `Primary(guard)`: we own the singleton; install the receiver
///   window and run the event loop. The guard's `Drop` releases
///   the mutex on process exit.
/// - `Secondary`: another `jian.exe` is running; forward our argv
///   via [`forward_url_to_primary`] and exit.
pub enum Singleton {
    /// Held the named mutex; this process is the running primary.
    /// Drop releases the OS handle.
    Primary(SingletonGuard),
    /// Another `jian.exe` already holds the mutex.
    Secondary,
}

/// RAII handle around the named mutex + the receiver-ready
/// named event. The event is created at singleton-acquisition
/// time (NOT at window-install time) so secondaries that arrive
/// during the primary's startup work find a live event object
/// to wait on (codex round 3 MEDIUM Q4 #1: a previous design
/// only created the event inside `install_receiver_window`,
/// leaving a window where secondaries' `OpenEventW` returned
/// null and they fell straight through to `FindWindowExW` —
/// returning `NoPeer` even though the primary was about to
/// come up). The event starts unsignaled; the primary's
/// `install_receiver_window` calls `SetEvent` after the HWND is
/// live.
///
/// `Drop` closes both handles so the next `jian.exe` startup
/// sees the slot free.
pub struct SingletonGuard {
    mutex: HANDLE,
    /// Manual-reset event the primary signals after its receiver
    /// window is installed. Held here (rather than inside
    /// `ReceiverWindow`) so it predates window install.
    ready_event: HANDLE,
}

impl SingletonGuard {
    /// Borrow the ready-event handle so `install_receiver_window`
    /// can `SetEvent` after the HWND is up. Returned as a raw
    /// `HANDLE` to keep the public API surface narrow.
    pub fn ready_event(&self) -> HANDLE {
        self.ready_event
    }
}

impl Drop for SingletonGuard {
    fn drop(&mut self) {
        // SAFETY: both handles came from `CreateMutexW` /
        // `CreateEventW`. Closing releases the kernel-object
        // reference. Return values ignored — there's no recovery
        // for a teardown-time close failure.
        if !self.mutex.is_null() {
            unsafe {
                let _ = CloseHandle(self.mutex);
            }
        }
        if !self.ready_event.is_null() {
            unsafe {
                let _ = CloseHandle(self.ready_event);
            }
        }
    }
}

/// Try to claim the named-mutex singleton. Returns
/// [`Singleton::Primary`] if no other `jian.exe` is running
/// (mutex created fresh) or [`Singleton::Secondary`] if a peer
/// already holds it (`GetLastError() == ERROR_ALREADY_EXISTS`).
///
/// Mutex creation failure (any other error) is treated as
/// "secondary" too — falling back to forwarding is safer than
/// running a second-window primary and confusing the user.
///
/// # Safety
///
/// Calls into the Win32 mutex API with no shared state from
/// outside this module. Safe to call from any thread, but
/// idiomatically called once on the main thread before the
/// event loop spins up.
pub fn try_acquire_singleton() -> Singleton {
    let mutex_name = utf16_with_nul(SINGLETON_MUTEX_NAME);
    // SAFETY: name pointer is valid for the call; CreateMutexW
    // accepts NULL for default security.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if mutex.is_null() {
        // Couldn't even create the kernel object — treat as
        // secondary so the caller forwards instead of running.
        return Singleton::Secondary;
    }
    let last = unsafe { GetLastError() };
    if last == ERROR_ALREADY_EXISTS {
        // Peer holds the mutex; release our handle (the mutex
        // itself stays alive in the kernel as long as the peer
        // holds it) and return Secondary.
        unsafe {
            let _ = CloseHandle(mutex);
        }
        return Singleton::Secondary;
    }
    // Primary path: pre-create the receiver-ready event NOW
    // (codex round 3 MEDIUM) so secondaries arriving while we're
    // still in startup find a live event to wait on. Manual-
    // reset, initially unsignaled. `install_receiver_window`
    // signals it after the HWND is up.
    let event_name = utf16_with_nul(READY_EVENT_NAME);
    let ready_event = unsafe {
        CreateEventW(
            std::ptr::null(),
            1, /* manual reset */
            0,
            event_name.as_ptr(),
        )
    };
    if ready_event.is_null() {
        // Event creation failed — log + continue with mutex only.
        // Secondaries will fall through to FindWindowExW directly,
        // which works once our window is up but loses the
        // bounded-wait guarantee for a brief window during startup.
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "jian-host-desktop: CreateEventW for receiver-ready signal failed; \
             secondary instances may experience a brief startup-window forwarding gap"
        );
    }
    Singleton::Primary(SingletonGuard { mutex, ready_event })
}

/// Hidden-window guard. Owned by the primary instance for the
/// lifetime of its event loop. `Drop` destroys the HWND so a
/// subsequent restart can re-register the class cleanly. The
/// class atom itself is intentionally NOT unregistered (Windows
/// unregisters EXE-local classes at process exit; re-registration
/// by the same name returns the existing atom).
///
/// Note: the receiver-ready event lives on `SingletonGuard` (not
/// here) so it's created at singleton-acquisition time and outlives
/// any window-install retry.
pub struct ReceiverWindow {
    hwnd: HWND,
}

// Codex round 2 MEDIUM: `ReceiverWindow` deliberately is NOT
// `Send`/`Sync`. `DestroyWindow` is thread-affine — it must run on
// the thread that called `CreateWindowExW`. Sending the guard to
// another thread would silently no-op the destroy and orphan the
// HWND. The thread-local `RECEIVER_WINDOW` storage already pins
// it to the creator thread; downstream callers must keep that
// invariant.

impl Drop for ReceiverWindow {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            // SAFETY: hwnd came from `CreateWindowExW`; destroying
            // releases the OS-side window resources. Return value
            // ignored — teardown-time failures aren't recoverable.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// Memoised outcome of the one-shot `RegisterClassExW` call.
///
/// Codex round 2 MEDIUM: a previous `OnceLock<()>` design always
/// stored the unit value the first time `get_or_init` ran,
/// regardless of whether the registration succeeded. A transient
/// `RegisterClassExW` failure would leave the OnceLock memoised
/// as "ran" and every subsequent call would silently skip the
/// registration. Storing `Ok(())` on success / duplicate-class
/// or `Err(msg)` on other failures lets the caller observe the
/// real outcome on every call. Subsequent calls re-read the same
/// `Err`; the host's caller doesn't get to retry with the same
/// process (which is the correct semantic — `RegisterClassExW`
/// only fails for resource exhaustion that won't fix itself
/// within one process lifetime), but it does get a clean error
/// instead of a silent skip.
static CLASS_REGISTRATION: OnceLock<Result<(), &'static str>> = OnceLock::new();

/// Named event the primary `SetEvent`s once its receiver window
/// is created and ready. Secondaries `WaitForSingleObject` on it
/// before calling `FindWindowExW` so a brief startup window doesn't
/// drop their forward. Same `Local\` namespace as the singleton
/// mutex (see `SINGLETON_MUTEX_NAME`).
const READY_EVENT_NAME: &str = r"Local\JianHostDesktop-ReceiverReady";

/// Register the `WNDCLASSEXW`, create a `HWND_MESSAGE` window with
/// our custom `WindowProc`, and signal the singleton's pre-created
/// "ready" event so secondary instances know forwarding is safe.
/// Pass the `SingletonGuard` so this fn can `SetEvent` the handle
/// the singleton already owns (codex round 3 MEDIUM: the event
/// must outlive any retry of `install_receiver_window`).
///
/// `RegisterClassExW` failures other than `ERROR_CLASS_ALREADY_EXISTS`
/// surface as an `Err` (codex round 1 MEDIUM Q3) so a transient
/// resource failure doesn't get cached as "registered" and stop a
/// retry from working.
///
/// # Safety
///
/// Internally calls `RegisterClassExW` + `CreateWindowExW` +
/// `SetEvent`, all `unsafe` for FFI reasons. The function as a
/// whole is safe to call once on the main thread — `WindowProc`
/// runs on the message-pump thread (winit pumps on main), so
/// receiver-window messages process alongside regular
/// WM_PAINT / WM_INPUT traffic.
pub fn install_receiver_window(singleton: &SingletonGuard) -> Result<ReceiverWindow, &'static str> {
    let class_name = utf16_with_nul(crate::win_deeplink::RECEIVER_CLASS_NAME);
    // Compute the module handle ONCE and use it for both
    // class registration and `CreateWindowExW`. Codex round 1
    // MEDIUM Q5: passing NULL as `hInstance` to `CreateWindowExW`
    // while the class is registered with `GetModuleHandleW(NULL)`
    // makes the class lookup fail (Windows scopes class lookups
    // by module instance).
    let hinstance: HMODULE = unsafe { GetModuleHandleW(std::ptr::null()) };

    // Register the class exactly once per process. The closure's
    // return value is memoised: `Ok(())` on success or duplicate-
    // class, `Err(msg)` on any other RegisterClassExW failure.
    // Subsequent calls observe the same outcome (codex round 2
    // MEDIUM — a previous design lost this).
    let outcome: &Result<(), &'static str> = CLASS_REGISTRATION.get_or_init(|| {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(window_proc_imp),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0 {
            let last = unsafe { GetLastError() };
            if last == ERROR_CLASS_ALREADY_EXISTS {
                Ok(()) // duplicate-name → fine, reuse existing atom
            } else {
                Err("RegisterClassExW failed with non-duplicate error \
                     (resource exhaustion?)")
            }
        } else {
            Ok(())
        }
    });
    if let Err(msg) = outcome {
        return Err(*msg);
    }

    // SAFETY: class registered with `hinstance`; HWND_MESSAGE is
    // the documented sentinel parent for message-only windows.
    // Window name reuses the class name (informational only;
    // `FindWindowExW` filters by class).
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            HWND_MESSAGE,
            std::ptr::null_mut::<HMENU>() as _,
            hinstance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW returned NULL — receiver window not created");
    }

    // Signal the singleton's pre-created ready event so any
    // secondary already waiting on it wakes up. Manual-reset
    // semantics mean later-arriving secondaries also see it
    // signaled. NULL ready_event indicates `try_acquire_singleton`
    // hit a `CreateEventW` failure earlier; in that case
    // secondaries will fall through to FindWindowExW directly.
    let ready_event = singleton.ready_event();
    if !ready_event.is_null() {
        unsafe {
            let _ = SetEvent(ready_event);
        }
    }
    Ok(ReceiverWindow { hwnd })
}

/// Maximum time the secondary will wait on the named ready event
/// before giving up and either calling `FindWindowExW` directly
/// (in case the primary's CreateEventW failed but the window
/// exists anyway) or returning `Ok(false)` so the caller's fallback
/// runs. 5 s is generous for a normal Windows startup; primary's
/// path from `try_acquire_singleton` to `install_receiver_window`
/// is bounded by argument parsing + a single window create.
const READY_WAIT_MS: u32 = 5_000;

/// Maximum time `SendMessageTimeoutW` blocks waiting for the
/// primary's `WindowProc` to ack the `WM_COPYDATA`. Plain
/// `SendMessageW` would block indefinitely if the receiver thread
/// is hung or a spoofed peer never pumps; the timeout bounds it.
///
/// We pass `SMTO_BLOCK` only (not `SMTO_ABORTIFHUNG`). Codex round
/// 4 MEDIUM: ABORTIFHUNG can fire when the OS' "thread looks hung"
/// heuristic decides a primary doing legitimate cold-start work
/// (file read, schema parse, runtime build — typically 30-100 ms)
/// hasn't pumped its message loop "fast enough", which would cut
/// off a secondary's forward unnecessarily. SMTO_BLOCK still
/// prevents the secondary from processing other messages while
/// it waits (we're about to exit anyway).
const SEND_TIMEOUT_MS: u32 = 5_000;

/// Outcome of [`forward_url_to_primary`]. Codex round 2 MEDIUM:
/// previously the function collapsed "no peer", "send timed out",
/// and "receiver returned 0" into a single `Ok(false)`, which the
/// CLI then misinterpreted as "no peer found" — a hung primary or
/// a payload-rejection looked indistinguishable from a missing
/// peer. The typed variants let the caller branch correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    /// Primary's `WindowProc` ack'd the WM_COPYDATA with a
    /// non-zero return — URL was accepted.
    Delivered,
    /// `FindWindowExW` returned NULL after waiting on the ready
    /// event, even though the singleton mutex was held. Most
    /// likely the primary's window install failed; the caller
    /// should refuse to start a second window (calling fallback
    /// would break the singleton invariant).
    NoPeer,
    /// `SendMessageTimeoutW` returned 0 with a timeout-style
    /// `GetLastError` (`ERROR_TIMEOUT` / 0). Primary's window
    /// exists but its message pump isn't dispatching within the
    /// budget. Caller semantic: refuse to start a second window.
    SendTimedOut,
    /// `SendMessageTimeoutW` returned 0 with a non-timeout error
    /// (access-denied / UIPI block / invalid-window / etc.).
    /// The secondary couldn't deliver the URL even though the
    /// HWND looked live; surfaces the underlying Win32 error
    /// code for diagnostic logging. Codex round 4 MEDIUM: a
    /// previous variant collapsed this with `SendTimedOut` so
    /// elevation / DACL mismatches were undiagnosable.
    SendFailed { last_error: u32 },
    /// Primary's `WindowProc` returned 0 — typically because
    /// `dispatch_url` rejected the URL (parse error, no handler).
    /// The URL was delivered in protocol terms but not "actioned";
    /// from the secondary's POV, this is "primary rejected" —
    /// NOT a fallback case. Logging up to the caller.
    PrimaryRejected,
}

/// Find the running primary's receiver window and forward `url`
/// via `WM_COPYDATA`. The returned [`ForwardOutcome`] tells the
/// caller whether to exit success, exit error, or escalate.
///
/// # Safety
///
/// Calls `OpenEventW` + `WaitForSingleObject` + `FindWindowExW` +
/// `SendMessageTimeoutW`. All operate on well-defined Win32
/// inputs; no shared state from outside this module is involved.
pub fn forward_url_to_primary(url: &str) -> Result<ForwardOutcome, &'static str> {
    // Codex round 1 MEDIUM Q1: wait on the named ready event so a
    // tiny startup gap between "primary acquired mutex" and
    // "primary's window listening" doesn't surface as a forward
    // failure. If the event doesn't exist (older primary that
    // didn't create it, or CreateEventW failed) we fall through
    // to the FindWindowExW probe directly.
    let event_name = utf16_with_nul(READY_EVENT_NAME);
    // Codex round 4 MEDIUM: secondary only needs `SYNCHRONIZE`
    // (it reads the signaled state via `WaitForSingleObject`, never
    // sets the state). Asking for `EVENT_MODIFY_STATE` could fail
    // under stricter DACL / mandatory-integrity-level setups where
    // the secondary's token doesn't grant write to a primary-
    // created event.
    let event_handle = unsafe { OpenEventW(SYNCHRONIZE, 0, event_name.as_ptr()) };
    if !event_handle.is_null() {
        let wait = unsafe { WaitForSingleObject(event_handle, READY_WAIT_MS) };
        unsafe {
            let _ = CloseHandle(event_handle);
        }
        // Codex round 2 MEDIUM Q3: explicit match on the three
        // documented `WaitForSingleObject` return codes. Plain
        // timeout falls through to FindWindowExW; an OS-level
        // failure propagates as Err.
        if wait == WAIT_FAILED {
            return Err("WaitForSingleObject(receiver-ready) failed");
        }
        // WAIT_OBJECT_0 → ready, proceed.
        // WAIT_TIMEOUT → primary still coming up; FindWindowExW
        //   below resolves "no peer" vs "raced past timeout".
        let _ = (wait == WAIT_OBJECT_0, wait == WAIT_TIMEOUT);
    }

    let class_name = utf16_with_nul(crate::win_deeplink::RECEIVER_CLASS_NAME);
    // SAFETY: HWND_MESSAGE filter restricts the search to
    // message-only windows; class name pinning keeps us from
    // matching arbitrary other apps' windows.
    let peer = unsafe {
        FindWindowExW(
            HWND_MESSAGE,
            std::ptr::null_mut(),
            class_name.as_ptr(),
            std::ptr::null(),
        )
    };
    if peer.is_null() {
        return Ok(ForwardOutcome::NoPeer);
    }

    // Encode URL as UTF-16 LE bytes (no NUL terminator — cbData
    // carries the byte count). Reject early if the URL exceeds
    // the receiver's `COPYDATA_MAX_BYTES` cap.
    let utf16: Vec<u16> = url.encode_utf16().collect();
    let cb_data: u32 = utf16
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("URL too long for COPYDATASTRUCT.cbData (u32)")?;
    if cb_data as usize > COPYDATA_MAX_BYTES {
        return Err("URL exceeds receiver's COPYDATA_MAX_BYTES cap");
    }
    let cds = COPYDATASTRUCT {
        dwData: COPYDATA_TAG,
        cbData: cb_data,
        lpData: utf16.as_ptr() as *mut _,
    };
    // SAFETY: peer is a valid HWND from FindWindowExW; cds points
    // to a stack value live for the duration of the call;
    // `SendMessageTimeoutW` is synchronous-blocking so utf16's
    // backing buffer survives the kernel-side copy. Codex round 1
    // MEDIUM Q2 liveness: bounded timeout (5 s) so a hung / spoofed
    // peer can't deadlock the secondary. See `SEND_TIMEOUT_MS` doc
    // for why we don't combine `SMTO_ABORTIFHUNG`.
    let mut result: usize = 0;
    // Codex round 4 MEDIUM: drop `SMTO_ABORTIFHUNG` so a primary
    // doing legitimate cold-start work (file read, schema parse,
    // runtime build) — which can take 30-100ms before the message
    // pump dispatches — isn't pre-empted by the OS' "thread looks
    // hung" heuristic. Plain timeout still fires at SEND_TIMEOUT_MS.
    let send_status = unsafe {
        SendMessageTimeoutW(
            peer,
            WM_COPYDATA,
            0_usize,
            &cds as *const _ as LPARAM,
            SMTO_BLOCK,
            SEND_TIMEOUT_MS,
            &mut result as *mut _,
        )
    };
    if send_status == 0 {
        // Codex round 4 MEDIUM: distinguish timeout vs send
        // failure (access-denied, UIPI, invalid-window) so the
        // caller can log a useful diagnostic.
        let last = unsafe { GetLastError() };
        return Ok(if last == 0 || last == ERROR_TIMEOUT {
            ForwardOutcome::SendTimedOut
        } else {
            ForwardOutcome::SendFailed { last_error: last }
        });
    }
    // Codex round 2 MEDIUM Q4#1: the receiver's WindowProc
    // returns 0 when `dispatch_url` failed (bad URL, no handler,
    // handler declined). Pre-fix, we returned Ok(true) regardless
    // — silently dropping rejected URLs.
    if result == 0 {
        return Ok(ForwardOutcome::PrimaryRejected);
    }
    Ok(ForwardOutcome::Delivered)
}

/// `WindowProc` for the receiver window. Handles `WM_COPYDATA`,
/// passes everything else to `DefWindowProcW`.
extern "system" fn window_proc_imp(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Codex Apple-Event lessons: a panic crossing this `extern
    // "system"` boundary is undefined behaviour, and dropping a
    // panic payload can re-panic. Wrap + mem::forget identically.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if msg != WM_COPYDATA {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // SAFETY: WM_COPYDATA carries `*const COPYDATASTRUCT` in
        // lParam (Windows API contract).
        let cds = lparam as *const COPYDATASTRUCT;
        if cds.is_null() {
            return 0;
        }
        let cds = unsafe { &*cds };
        if cds.dwData != COPYDATA_TAG {
            // Some other app spoofing WM_COPYDATA on our class —
            // drop silently. Class-name filter on FindWindowExW
            // should prevent this in practice; tag-check is
            // defense in depth.
            return 0;
        }
        // Validate payload alignment + size + non-null pointer +
        // upper-bound size before reinterpreting.
        // Codex round 1 HIGH: `COPYDATASTRUCT.lpData` is documented
        // nullable; spoofed `cbData > 0, lpData = NULL` would
        // drive `from_raw_parts(NULL, cbData/2)` — immediate UB.
        if cds.lpData.is_null() {
            return 0;
        }
        let expected_align = std::mem::align_of::<u16>();
        let bytes_per_unit = std::mem::size_of::<u16>();
        let cb = cds.cbData as usize;
        // Codex round 2 MEDIUM: cap cbData to defend against a
        // same-session attacker pushing huge allocations through
        // the public tag.
        if cb == 0 || cb % bytes_per_unit != 0 || cb > COPYDATA_MAX_BYTES {
            return 0;
        }
        if (cds.lpData as usize) % expected_align != 0 {
            // Misaligned payload — refuse to read u16s through it.
            return 0;
        }
        let units = cb / bytes_per_unit;
        // SAFETY: bounds + alignment validated above; the kernel-
        // side cross-process copy guarantees the buffer is live for
        // the duration of the SendMessageW call.
        let slice = unsafe { std::slice::from_raw_parts(cds.lpData as *const u16, units) };
        let url = match String::from_utf16(slice) {
            Ok(s) => s,
            Err(_) => return 0,
        };
        // Forward into the cross-platform deeplink dispatcher.
        match crate::win_deeplink::dispatch_url(&url) {
            Ok(_handled) => 1, // 1 = "I handled WM_COPYDATA"
            Err(e) => {
                use std::io::Write;
                let _ = writeln!(
                    std::io::stderr(),
                    "jian-host-desktop: dispatch_url failed at WM_COPYDATA boundary: {e}"
                );
                0
            }
        }
    }));
    match result {
        Ok(v) => v,
        Err(payload) => {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "jian-host-desktop: panic in DeepLinkHandler caught at WM_COPYDATA boundary; \
                 event dropped"
            );
            std::mem::forget(payload);
            0
        }
    }
}

/// Park the just-created receiver window in the thread-local so
/// its `Drop` doesn't fire when the install function returns.
/// Replacing an existing window guard returns the old one (the
/// host may want to defer-drop it after the new window is wired).
pub fn store_receiver_window(window: ReceiverWindow) -> Option<ReceiverWindow> {
    RECEIVER_WINDOW.with(|cell| cell.replace(Some(window)))
}

/// Drop the stored receiver window, if any. Used during host
/// teardown so the HWND is released before the process exits.
pub fn take_receiver_window() -> Option<ReceiverWindow> {
    RECEIVER_WINDOW.with(|cell| cell.borrow_mut().take())
}

/// UTF-16 encoding helper. Windows `FindWindowExW` /
/// `CreateWindowExW` / `RegisterClassExW` / `CreateMutexW` all
/// expect NUL-terminated wide strings.
fn utf16_with_nul(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copydata_tag_pinned() {
        // Wire-format pin: bumping the tag requires a coordinated
        // primary+secondary update in the same release. Same-tag
        // mismatch on the wire produces a silent drop, which is
        // deliberate (defense-in-depth against cross-app spoof)
        // but benign — the test catches an accidental change.
        assert_eq!(COPYDATA_TAG, 0x4A_44_4C_31);
        // The four bytes spell 'JDL1' in ASCII. Use FourCC math
        // (not byte-slice tricks — `usize` width varies across
        // 32-bit / 64-bit targets, codex round 2 missed this).
        let four_cc = ((b'J' as usize) << 24)
            | ((b'D' as usize) << 16)
            | ((b'L' as usize) << 8)
            | (b'1' as usize);
        assert_eq!(COPYDATA_TAG, four_cc);
    }

    #[test]
    fn singleton_mutex_name_is_pinned() {
        // Bumping the mutex name breaks the singleton contract
        // mid-rollout — primary on old name + secondary on new
        // name would both think they're primary. Same-release
        // updates only. `Local\` namespace pairs with the session-
        // scoped HWND_MESSAGE window discovery (codex round 2 MEDIUM).
        assert_eq!(SINGLETON_MUTEX_NAME, r"Local\JianHostDesktop-Singleton");
    }

    #[test]
    fn ready_event_name_uses_same_session_namespace_as_mutex() {
        // Both must share the `Local\` prefix so they target the
        // same session — pairing them with a session-scoped
        // HWND_MESSAGE forwarder.
        assert!(SINGLETON_MUTEX_NAME.starts_with(r"Local\"));
        assert!(READY_EVENT_NAME.starts_with(r"Local\"));
    }

    #[test]
    fn copydata_max_bytes_fits_realistic_urls() {
        // RFC URLs realistically top at ~2 KiB; 4 KiB cap leaves
        // headroom but rejects pathological allocations.
        assert_eq!(COPYDATA_MAX_BYTES, 4 * 1024);
    }

    #[test]
    fn utf16_helper_appends_nul() {
        let v = utf16_with_nul("ab");
        assert_eq!(v, vec![b'a' as u16, b'b' as u16, 0]);
    }
}
