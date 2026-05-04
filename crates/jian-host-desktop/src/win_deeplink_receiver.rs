//! Windows hidden message-only window + named-mutex single-instance
//! plumbing (Plan 8 §T8). Pairs with macOS's
//! [`crate::apple_event_receiver`].
//!
//! ## Flow (Plan 8 §T8 follow-up B — pipe transport)
//!
//! 1. **Primary instance**: at startup, calls
//!    [`try_acquire_singleton`]. The named `CreateMutexW` succeeds
//!    with no `ERROR_ALREADY_EXISTS`, so we own the mutex for the
//!    process lifetime. Then [`install_receiver_window`] registers
//!    a `WNDCLASSEXW` named
//!    [`crate::win_deeplink::RECEIVER_CLASS_NAME`], creates a
//!    `HWND_MESSAGE` window, and spawns the per-user named-pipe
//!    listener thread via
//!    [`crate::win_deeplink_pipe::install_pipe_listener`]. The
//!    listener owns the pipe handle for its lifetime and feeds
//!    incoming URLs to the receiver HWND through
//!    `WM_USER_DEEPLINK_FORWARD` `PostMessageW`s.
//! 2. **Secondary instance** (same `jian.exe`, second click on a
//!    `jian://...` URL): [`try_acquire_singleton`] returns
//!    [`Singleton::Secondary`]. The CLI then calls
//!    [`forward_url_to_primary`], which (a) waits on the
//!    [`READY_EVENT_NAME`] event so a startup-gap forward isn't
//!    spuriously rejected, then (b) opens the per-user pipe via
//!    `CreateFileW(\\.\pipe\jian-deeplink-<sid>, GENERIC_WRITE)`
//!    and writes `url + '\n'`. The DACL on the pipe authenticates
//!    the secondary by user SID — a different-user secondary
//!    sees `ERROR_ACCESS_DENIED`. The CLI then exits success.
//! 3. The primary's listener thread reads the URL line, packages
//!    it in a heap-leaked `Box<String>`, and `PostMessageW`s the
//!    receiver HWND. The receiver `WindowProc` recovers the Box
//!    and feeds the URL into [`crate::win_deeplink::dispatch_url`].
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
//! --target x86_64-pc-windows-msvc` compile-checks the FFI
//! surface, but cross-process delivery only exercises against a
//! live process pair. Unit tests below pin the singleton + ready-
//! event names; the pipe-transport tests in
//! [`crate::win_deeplink_pipe`] pin the pipe-name shape and the
//! `WM_USER_DEEPLINK_FORWARD` constant.
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
//! ## Cross-process delivery: named-pipe transport (Plan 8 §T8 follow-up B)
//!
//! Round-1 used `FindWindowExW` + `SendMessageTimeoutW` +
//! `WM_COPYDATA`, which authenticated the peer only by window-
//! class string — a same-session attacker registering the same
//! class could intercept. **The current implementation uses a
//! named-pipe transport** ([`crate::win_deeplink_pipe`]) whose
//! DACL is restricted to the calling user's SID:
//!
//! - Pipe name: `\\.\pipe\jian-deeplink-<user_sid>` — per-user so
//!   two users on the same Windows host don't collide on the
//!   machine-global pipe namespace.
//! - DACL: `D:P(A;;GA;;;<user_sid>)` — `D:P` blocks inherited
//!   ACEs, the single Allow ACE grants `GENERIC_ALL` to exactly
//!   the creator's SID.
//! - `PIPE_REJECT_REMOTE_CLIENTS` declines off-box connections.
//! - `FILE_FLAG_FIRST_PIPE_INSTANCE` + `nMaxInstances = 1` keeps
//!   a duplicate `CreateNamedPipeW` from stealing the name.
//!
//! The receiver `WindowProc` no longer handles `WM_COPYDATA`.
//! Instead the pipe-listener thread (spawned at
//! `install_receiver_window` time) reads URL lines from the pipe
//! and `PostMessageW`s them to the receiver HWND with
//! [`crate::win_deeplink_pipe::WM_USER_DEEPLINK_FORWARD`]
//! carrying a heap-leaked `Box<String>` pointer in `lParam`.
//! WindowProc recovers the Box and feeds the URL into
//! [`crate::win_deeplink::dispatch_url`].

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
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_CLASS_ALREADY_EXISTS, HANDLE, HMODULE,
    HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
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
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, CW_USEDEFAULT, HMENU,
    HWND_MESSAGE, WNDCLASSEXW, WS_OVERLAPPED,
};

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
        // Secondaries will fall through to the pipe-probe directly,
        // which works once the listener is bound but loses the
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
/// before opening the pipe so a brief startup window doesn't
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

    // Plan 8 §T8 follow-up B: spawn the named-pipe listener
    // BEFORE signaling the ready event. If the pipe install
    // fails, surface as Err and DESTROY the receiver window so
    // we don't leave a half-installed primary that secondaries
    // mistake for live (HWND found by class name, but no pipe
    // accepting writes — secondaries would hit `NoPeer` and
    // refuse to start a second window, leaving the user with
    // no way to open the URL).
    //
    // Codex round 1 CONCERN: the in-process restart path
    // (host-tear-down without process exit) needs the listener
    // thread to observe shutdown and release the pipe before a
    // second `install_receiver_window` call; today the daemon
    // thread holds the pipe for the process lifetime, so a
    // re-install hits `FILE_FLAG_FIRST_PIPE_INSTANCE`'s
    // `ERROR_ACCESS_DENIED`. Surfacing the error means the host
    // sees a clean failure rather than a silent half-install.
    // True in-process restart support requires an overlapped-I/O
    // shutdown event on the listener thread, deferred to a
    // follow-up that touches the host's teardown sequence.
    if let Err(e) = crate::win_deeplink_pipe::install_pipe_listener(hwnd) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "jian-host-desktop: pipe listener install failed ({e}); receiver window \
             destroyed — deep-link forwarding refused for this primary"
        );
        return Err("pipe listener install failed");
    }

    // Signal the singleton's pre-created ready event so any
    // secondary already waiting on it wakes up. Manual-reset
    // semantics mean later-arriving secondaries also see it
    // signaled. NULL ready_event indicates `try_acquire_singleton`
    // hit a `CreateEventW` failure earlier; in that case
    // secondaries will fall through to the pipe probe directly.
    let ready_event = singleton.ready_event();
    if !ready_event.is_null() {
        unsafe {
            let _ = SetEvent(ready_event);
        }
    }
    Ok(ReceiverWindow { hwnd })
}

/// Maximum time the secondary will wait on the named ready event
/// before giving up and probing the pipe directly (in case the
/// primary's `CreateEventW` failed but the listener exists
/// anyway). 5 s is generous for a normal Windows startup;
/// primary's path from `try_acquire_singleton` to
/// `install_receiver_window` (which spawns the listener) is
/// bounded by argument parsing + a single window create + a
/// single pipe-bind.
const READY_WAIT_MS: u32 = 5_000;

/// Outcome of [`forward_url_to_primary`].
///
/// Plan 8 §T8 follow-up B note: with the named-pipe transport
/// the variant set is narrower than the round-1
/// `WM_COPYDATA`-over-`SendMessageTimeoutW` shape. A pipe write
/// that succeeds but the listener can't dispatch (handler
/// returns false / parse error / etc.) is effectively
/// `Delivered` from the secondary's POV — the listener thread
/// queued the URL and logged any failure to the primary's
/// stderr. Variants `SendTimedOut` and `PrimaryRejected` are
/// retained for source-compat with the CLI's existing branch
/// arms (mapped from `SendFailed` with specific error codes if
/// needed) but the pipe transport doesn't actually emit them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    /// `WriteFile` + `FlushFileBuffers` succeeded on the
    /// per-user pipe. Caller exits success.
    Delivered,
    /// `CreateFileW` returned `ERROR_FILE_NOT_FOUND` /
    /// `ERROR_PIPE_BUSY` / `ERROR_PIPE_NOT_CONNECTED` — the
    /// per-user pipe doesn't exist or isn't listening. Combined
    /// with a held singleton mutex this means the primary is
    /// running but its pipe listener thread isn't bound; the
    /// CLI should refuse to start a second window.
    NoPeer,
    /// **Deprecated — pipe transport never emits this.** Kept
    /// for source-compat with existing CLI branch arms; future
    /// refactor can remove it once the CLI uses an exhaustive
    /// `_ =>` on the `Result<ForwardOutcome>` instead.
    SendTimedOut,
    /// `CreateFileW` succeeded but `WriteFile` /
    /// `FlushFileBuffers` failed — surfaces the Win32 error code
    /// (access-denied on a cross-user secondary, broken pipe on
    /// a primary mid-shutdown, etc.).
    SendFailed { last_error: u32 },
    /// **Deprecated — pipe transport never emits this.** The
    /// listener-side dispatch failure is logged on the primary's
    /// stderr (the secondary doesn't see it). Same source-compat
    /// note as `SendTimedOut`.
    PrimaryRejected,
}

/// Forward `url` to the running primary via the named-pipe
/// transport (Plan 8 §T8 follow-up B). The returned
/// [`ForwardOutcome`] tells the caller whether to exit success,
/// exit error, or escalate.
///
/// Pre-pipe round-1 used `FindWindowExW` + `SendMessageTimeoutW`
/// + `WM_COPYDATA`, which authenticated the peer only by window-
/// class string — a same-session attacker registering the same
/// class could intercept. The pipe transport's user-SID DACL
/// closes that gap (only the calling user's processes can write
/// to the pipe), and `PIPE_REJECT_REMOTE_CLIENTS` blocks remote
/// pipe access on top.
///
/// We still wait on the [`READY_EVENT_NAME`] event so a forward
/// arriving during the primary's startup gap (mutex held but
/// pipe listener not yet bound) doesn't false-fail. If the wait
/// times out we fall through to the pipe probe; the pipe's own
/// `ERROR_FILE_NOT_FOUND` then resolves "no peer" cleanly.
///
/// # Safety
///
/// Internally calls `OpenEventW` + `WaitForSingleObject` + the
/// pipe-side `CreateFileW` / `WriteFile` / `FlushFileBuffers` /
/// `CloseHandle` chain. All operate on well-defined Win32 inputs.
pub fn forward_url_to_primary(url: &str) -> Result<ForwardOutcome, &'static str> {
    // Codex round 1 MEDIUM Q1: wait on the named ready event so a
    // tiny startup gap between "primary acquired mutex" and
    // "primary's listener bound" doesn't surface as a forward
    // failure. The event is signaled by `install_receiver_window`
    // AFTER the receiver HWND is up — and the pipe listener is
    // installed alongside the receiver, so the same signal
    // covers both paths.
    let event_name = utf16_with_nul(READY_EVENT_NAME);
    let event_handle = unsafe { OpenEventW(SYNCHRONIZE, 0, event_name.as_ptr()) };
    if !event_handle.is_null() {
        let wait = unsafe { WaitForSingleObject(event_handle, READY_WAIT_MS) };
        unsafe {
            let _ = CloseHandle(event_handle);
        }
        if wait == WAIT_FAILED {
            return Err("WaitForSingleObject(receiver-ready) failed");
        }
        // WAIT_OBJECT_0 → ready, proceed.
        // WAIT_TIMEOUT → primary still coming up; the pipe probe
        //   below resolves "no peer" cleanly.
        let _ = (wait == WAIT_OBJECT_0, wait == WAIT_TIMEOUT);
    }

    // Plan 8 §T8 follow-up B: forward via named pipe. The pipe's
    // user-SID DACL is the security boundary; a cross-user
    // sender hits access-denied here.
    use crate::win_deeplink_pipe::{forward_url_via_pipe, PipeForwardOutcome};
    match forward_url_via_pipe(url)? {
        PipeForwardOutcome::Delivered => Ok(ForwardOutcome::Delivered),
        PipeForwardOutcome::NoPeer => Ok(ForwardOutcome::NoPeer),
        PipeForwardOutcome::SendFailed { last_error } => {
            Ok(ForwardOutcome::SendFailed { last_error })
        }
    }
}

/// `WindowProc` for the receiver window. Handles
/// `WM_USER_DEEPLINK_FORWARD` (Plan 8 §T8 follow-up B — the
/// listener thread on the pipe transport posts here when a URL
/// arrives), passes everything else to `DefWindowProcW`.
///
/// Round-1 `WM_COPYDATA` is no longer received from
/// out-of-process secondaries — the named-pipe transport's DACL
/// gates cross-process delivery to the calling user only, and
/// the listener thread is the only inbound channel. A stray
/// `WM_COPYDATA` from a same-session attacker still falls through
/// to `DefWindowProcW`, which is a benign no-op.
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
        if msg != crate::win_deeplink_pipe::WM_USER_DEEPLINK_FORWARD {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // The listener thread leaked a `Box<String>` into lParam;
        // recover it here. Box::from_raw → owned String → drop
        // frees the heap allocation. NULL lParam shouldn't happen
        // (the listener never posts NULL) but defend against it.
        if lparam == 0 {
            return 0;
        }
        // SAFETY: lParam is a `*mut String` we leaked via
        // `Box::into_raw` in the listener thread (single producer
        // per HWND; PostMessage moves ownership to us). Recover
        // exactly once — the receiving WindowProc is the only
        // consumer of this lParam contract.
        let url_box: Box<String> = unsafe { Box::from_raw(lparam as *mut String) };
        let url: String = *url_box;
        // Forward into the cross-platform deeplink dispatcher.
        match crate::win_deeplink::dispatch_url(&url) {
            Ok(_handled) => 1, // 1 = "I handled the message"
            Err(e) => {
                use std::io::Write;
                let _ = writeln!(
                    std::io::stderr(),
                    "jian-host-desktop: dispatch_url failed at \
                     WM_USER_DEEPLINK_FORWARD boundary: {e}"
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
                "jian-host-desktop: panic in DeepLinkHandler caught at \
                 WM_USER_DEEPLINK_FORWARD boundary; event dropped"
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
    fn utf16_helper_appends_nul() {
        let v = utf16_with_nul("ab");
        assert_eq!(v, vec![b'a' as u16, b'b' as u16, 0]);
    }
}
