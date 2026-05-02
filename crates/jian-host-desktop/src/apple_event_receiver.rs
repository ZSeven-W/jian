//! macOS `kAEGetURL` Apple-Event receiver (Plan 8 §T8).
//!
//! Wires `[NSAppleEventManager sharedAppleEventManager]
//! setEventHandler:andSelector:forEventClass:andEventID:` to a
//! custom `NSObject` subclass that forwards the URL into
//! [`super::app_delegate::dispatch_url`]. With this in place,
//! `open jian://demo.counter/path` (terminal) or a `jian://`
//! click in another app routes through the running host instead
//! of the OS' default handler.
//!
//! ## Why NSAppleEventManager and not NSApplicationDelegate
//!
//! winit owns `NSApp.delegate`. Replacing it would either fork
//! winit or smuggle a Cocoa proxy that forwards every selector
//! winit cares about — both fragile. NSAppleEventManager registers
//! a separate dispatch path keyed on `(eventClass, eventID)`, so
//! the URL handler co-exists with winit's delegate without
//! contention. This is the same pattern Apple's own `URL` sample
//! ships in Xcode's "command-line app + URL scheme" template.
//!
//! ## Lifetime
//!
//! NSAppleEventManager retains the registered handler object
//! internally, but losing the Rust-side `Retained` reference would
//! cause the receiver to be released the next time AppKit calls a
//! method on it. We park the receiver in a thread-local so it lives
//! exactly as long as the process — same lifetime as the deep-link
//! handler registry in [`super::app_delegate`].
//!
//! ## Validation
//!
//! Runtime validation needs a real macOS GUI session: the AppleEvent
//! pipeline only fires for real `open jian://...` invocations
//! against a live `NSApplication`. The unit tests below cover the
//! pure-Rust slices (URL parsing forwarding, registry state) but
//! the OS hand-off necessarily exercises only at runtime in a
//! `cargo bundle`-built `.app` or via `open -a Jian.app jian://...`
//! manually.

#![cfg(target_os = "macos")]

use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, Sel};
use objc2::sel;
use objc2::{class, msg_send, msg_send_id};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSString};
use std::cell::RefCell;
use std::sync::OnceLock;

/// Four-character code packed as `u32`. AppleEvents identify event
/// classes / IDs / parameter keywords by these big-endian-encoded
/// FourCharCodes; in C the convention is `'GURL' == 0x4755524C`
/// (`'G' << 24 | 'U' << 16 | 'R' << 8 | 'L'`).
const fn four_cc(b: &[u8; 4]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

/// `kInternetEventClass = 'GURL'` — the AppleEvent class that
/// CoreServices uses to deliver URL-scheme dispatches.
const K_INTERNET_EVENT_CLASS: u32 = four_cc(b"GURL");
/// `kAEGetURL = 'GURL'` — the AppleEvent ID that pairs with the
/// class above for "please open this URL". Apple's headers use
/// the same FourCC for both, by design.
const K_AE_GET_URL: u32 = four_cc(b"GURL");
/// `keyDirectObject = '----'` — the parameter keyword that carries
/// the URL string for `kAEGetURL`.
const KEY_DIRECT_OBJECT: u32 = four_cc(b"----");

thread_local! {
    /// Holds the registered receiver object so `Retained` keeps it
    /// alive past the call to `setEventHandler:`. Without this,
    /// the receiver would drop right after registration and the
    /// AppleEvent dispatch would land on freed memory the first
    /// time `open jian://...` fires.
    static RECEIVER_HOLDER: RefCell<Option<Retained<NSObject>>> = const { RefCell::new(None) };
}

/// Registers the receiver class lazily — `NSObject` subclassing
/// must happen exactly once per process or the runtime aborts.
static RECEIVER_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

/// Install the `kAEGetURL` Apple-Event handler. Idempotent across
/// the process — the underlying NSObject subclass is registered
/// exactly once and the handler binding is reset each call.
///
/// Call this AFTER [`super::app_delegate::install_handler`] so a
/// URL arriving immediately on launch finds a registered receiver.
/// `[NSApp finishLaunching]` may fire `kAEGetURL` synchronously when
/// the process was started by a `jian://` click, so the order is
/// load-bearing.
///
/// Class registration failure panics (the only realistic cause is a
/// name conflict with another component in the same process — a
/// program-startup invariant that's better surfaced loudly than
/// threaded through a never-recoverable `Result`).
///
/// **Must be called from the main thread.** Cocoa fires
/// `application:openURLs:` on the main thread, and
/// `NSAppleEventManager`'s registration table is itself a main-
/// thread-only state machine — registering from a worker thread
/// produces obscure crashes deep in `dispatch_main` later. The
/// function asserts the precondition at entry (codex round 4
/// MEDIUM).
///
/// # Safety
///
/// Internally calls `objc2::msg_send!` to wire the handler, which is
/// `unsafe` for the standard Objective-C reasons. The function as a
/// whole is safe to call repeatedly from the main thread.
pub fn install_apple_event_handler() {
    assert_on_main_thread("install_apple_event_handler");
    let class = receiver_class();
    // `new` is a class method that does alloc+init in one shot.
    // Returns a `Retained<NSObject>` we park in the thread-local
    // so it outlives this call.
    let receiver: Retained<NSObject> = unsafe { msg_send_id![class, new] };

    // Borrow the receiver's raw pointer for the manager call. The
    // manager doesn't retain in older macOS releases (<10.7); modern
    // macOS does, but parking the receiver in the thread-local is
    // belt-and-braces.
    let receiver_ptr: *mut AnyObject = Retained::as_ptr(&receiver) as *mut AnyObject;
    let manager: Retained<NSAppleEventManager> =
        unsafe { NSAppleEventManager::sharedAppleEventManager() };
    let sel: Sel = sel!(handleURLEvent:withReplyEvent:);
    unsafe {
        let _: () = msg_send![
            &*manager,
            setEventHandler: receiver_ptr,
            andSelector: sel,
            forEventClass: K_INTERNET_EVENT_CLASS,
            andEventID: K_AE_GET_URL,
        ];
    }

    RECEIVER_HOLDER.with(|cell| {
        *cell.borrow_mut() = Some(receiver);
    });
}

/// Drop the registered handler. Used by tests + the host's
/// shutdown path so a follow-up `install_apple_event_handler` sees
/// a fresh registry entry (NSAppleEventManager keeps its handler
/// table forever otherwise — fine for a process lifetime, less
/// fine for a single-process test sequence).
///
/// **Must be called from the main thread**, same reason as
/// [`install_apple_event_handler`] (codex round 4 MEDIUM).
pub fn uninstall_apple_event_handler() {
    assert_on_main_thread("uninstall_apple_event_handler");
    let manager: Retained<NSAppleEventManager> =
        unsafe { NSAppleEventManager::sharedAppleEventManager() };
    unsafe {
        let _: () = msg_send![
            &*manager,
            removeEventHandlerForEventClass: K_INTERNET_EVENT_CLASS,
            andEventID: K_AE_GET_URL,
        ];
    }
    RECEIVER_HOLDER.with(|cell| {
        cell.borrow_mut().take();
    });
}

/// The `handleURLEvent:withReplyEvent:` body. AppKit calls this
/// (via the registered selector) when CoreServices delivers a
/// `kAEGetURL` event to the running app. Pulls the URL out of the
/// event's direct-object parameter and forwards into the deeplink
/// dispatcher.
extern "C" fn handle_url_event_imp(
    _self: *mut AnyObject,
    _sel: Sel,
    event: *mut AnyObject,
    _reply: *mut AnyObject,
) {
    // Codex round 2 MEDIUM: a panic in the user-supplied
    // DeepLinkHandler that crosses this `extern "C"` boundary is
    // undefined behaviour (the unwinder can't unwind across a non-
    // Rust frame). Wrap the dispatch in `catch_unwind` so a panic
    // becomes a dropped event rather than a process-wide abort.
    //
    // Codex round 3 MEDIUM: dropping the caught panic payload can
    // itself panic (per `std::panic::catch_unwind`'s docs), and
    // that secondary unwind would re-cross the FFI frame. Use
    // `mem::forget` on the payload so its `Drop` never runs.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if event.is_null() {
            return;
        }
        // SAFETY: the AppleEvent runtime guarantees `event` is an
        // NSAppleEventDescriptor live for the duration of the
        // callback.
        let event: &NSAppleEventDescriptor = unsafe { &*(event as *const NSAppleEventDescriptor) };
        let url_str = match extract_url_string(event) {
            Some(s) => s,
            None => return,
        };
        // Codex round 5 MEDIUM: surface BadUrl / NoHandlerRegistered
        // as a panic-safe stderr line rather than silently swallowing.
        // `writeln!` returns `Result` (no implicit panic on I/O fail);
        // `eprintln!` would panic on broken-pipe and unwind out of
        // the FFI frame.
        match super::app_delegate::dispatch_url(&url_str) {
            Ok(_handled) => {}
            Err(e) => {
                use std::io::Write;
                let _ = writeln!(
                    std::io::stderr(),
                    "jian-host-desktop: dispatch_url failed at AppleEvent boundary: {e}"
                );
            }
        }
    }));
    if let Err(payload) = result {
        // Codex round 4 MEDIUM: emit a diagnostic before forgetting
        // the payload. Codex round 5 MEDIUM: must NOT use
        // `eprintln!` — it panics on stderr write failure (broken
        // pipe etc.), and that secondary unwind would re-cross the
        // FFI frame. `writeln!(io::stderr(), ...)` returns `Result`
        // and is panic-safe; we discard the Err. We deliberately
        // don't try to format the payload (`Box<dyn Any + Send>`
        // doesn't implement Display, and downcasting could itself
        // hit user-supplied types).
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "jian-host-desktop: panic in DeepLinkHandler caught at AppleEvent boundary; \
             event dropped"
        );
        // Drop the payload via `forget` so a panicking destructor
        // doesn't re-cross the FFI boundary (codex round 3 MEDIUM).
        std::mem::forget(payload);
    }
}

/// Panic if not on the main thread. Used by the install /
/// uninstall entry points where Cocoa demands a main-thread caller.
/// Uses `pthread_main_np` from libc — unconditionally available on
/// Darwin and a 1-cycle check that doesn't drag in objc2-foundation's
/// `NSThread` API.
fn assert_on_main_thread(fn_name: &str) {
    extern "C" {
        fn pthread_main_np() -> i32;
    }
    // SAFETY: `pthread_main_np` is a leaf C function with no inputs
    // / outputs requiring lifetime correctness. Returns non-zero on
    // the main thread, zero otherwise.
    let main = unsafe { pthread_main_np() };
    assert!(
        main != 0,
        "jian-host-desktop::apple_event_receiver::{fn_name} called off the main thread; \
         NSAppleEventManager registration must run on the main thread"
    );
}

/// Pull the URL string out of an `NSAppleEventDescriptor`'s
/// `keyDirectObject` parameter. Returns `None` for events that
/// arrive without the expected parameter or whose parameter is
/// not a string.
fn extract_url_string(event: &NSAppleEventDescriptor) -> Option<String> {
    // `paramDescriptorForKeyword:` returns an `NSAppleEventDescriptor`
    // (or nil) for the named parameter. objc2-foundation 0.2.2's
    // generated bindings don't expose this method, so we go through
    // raw msg_send!. The returned object is an autoreleased
    // descriptor; we wrap it in a `Retained` to keep the borrow
    // checker happy until we extract the string.
    let param: *mut AnyObject =
        unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
    if param.is_null() {
        return None;
    }
    let descriptor: &NSAppleEventDescriptor = unsafe { &*(param as *const NSAppleEventDescriptor) };
    let ns_str: Option<Retained<NSString>> = unsafe { descriptor.stringValue() };
    ns_str.map(|s| s.to_string())
}

/// Lazily register the `JianAppleEventReceiver` `NSObject` subclass.
/// Uses `OnceLock::get_or_init` so allocation + `add_method` +
/// `register` all happen inside the lock's critical section. Codex
/// round 1 MEDIUM: a check-then-set pair would let two concurrent
/// first-time callers both reach `ClassBuilder::register` for the
/// same name, which the objc runtime aborts on. Every realistic
/// caller is on the main thread, but the lock makes the contract
/// explicit.
///
/// The closure panics on the only failure mode (`ClassBuilder::new`
/// returning `None`, which means the class name was already
/// registered by something else in this process). That's a one-shot
/// program-startup invariant — failing it loudly is more useful
/// than threading a `Result` through a never-recoverable error.
fn receiver_class() -> &'static AnyClass {
    *RECEIVER_CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut builder = ClassBuilder::new("JianAppleEventReceiver", superclass)
            .expect("JianAppleEventReceiver class already registered by another component");
        // `extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject)`
        // is the canonical IMP signature for an Objective-C method
        // taking two object arguments. `ClassBuilder::add_method`
        // synthesises the encoding (`v@:@@`) from the signature.
        let sel = sel!(handleURLEvent:withReplyEvent:);
        unsafe {
            builder.add_method(
                sel,
                handle_url_event_imp
                    as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
        }
        builder.register()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_cc_packs_big_endian() {
        // The constants here are the AppleEvent FourCharCodes the
        // C headers use. Pinning them prevents a future endianness
        // mistake.
        assert_eq!(K_INTERNET_EVENT_CLASS, 0x4755524C); // 'GURL'
        assert_eq!(K_AE_GET_URL, 0x4755524C); // 'GURL'
        assert_eq!(KEY_DIRECT_OBJECT, 0x2D2D2D2D); // '----'
    }
}
