//! App-icon abstraction for the desktop window (Plan 8 T8 polish).
//!
//! `app.icon: Option<String>` already lives on the schema's
//! `AppConfig` (`jian_ops_schema::app::AppConfig::icon`). This module
//! supplies the runtime-side plumbing to honour it at window-creation
//! time:
//!
//! - [`AppIcon`] is a pixel buffer in canonical RGBA8 form ready for
//!   `winit::window::Icon::from_rgba`. Pure data, no winit dependency.
//! - [`AppIconLoader`] is a trait hosts implement to turn the schema's
//!   string (a path, a URL, an embedded base64 data URI, …) into an
//!   `AppIcon`. The schema doesn't dictate which decoders the host
//!   supports; the trait keeps the runtime decoupled from `image-rs`,
//!   `png`, `ico`, etc.
//! - [`NullAppIconLoader`] is the no-op default that always reports
//!   "no decoder configured" — hosts that don't yet wire a real
//!   loader use this so the trait surface stays uniform across
//!   "decoders wired" and "decoders not wired" builds.
//!
//! Per-platform packaging-side icons (macOS `.icns` in
//! `Contents/Resources/`, Windows `.ico` embedded via `winres`, Linux
//! `.png` referenced from `.desktop`) live in Plan 8 Task 10
//! packaging — that's a separate follow-up. This module covers the
//! **runtime window icon** only.
//!
//! ### Per-platform support note
//!
//! `winit::window::Window::set_window_icon` is documented as
//! **unsupported on macOS and Wayland** (winit ≥ 0.30, see
//! <https://docs.rs/winit/latest/winit/window/struct.Window.html#method.set_window_icon>).
//! On those platforms the runtime icon is a no-op; the app's
//! Dock / launcher icon comes from the `.app` bundle's
//! `Contents/Resources/Foo.icns` (macOS) or the `.desktop` file's
//! `Icon=` reference (Wayland session shells), both of which are
//! Plan 8 Task 10's concern. On Windows + X11 the runtime icon DOES
//! show in the taskbar / titlebar.

use std::fmt;

/// Decoded icon in canonical RGBA8 form.
///
/// `rgba.len() == width * height * 4`. Color order is R, G, B, A;
/// alpha is straight (NOT premultiplied) — winit's
/// [`winit::window::Icon::from_rgba`] expects this shape and the
/// builder validates it.
///
/// `source_png` optionally carries the original PNG file bytes when
/// the loader read them from disk. This is the macOS Dock-icon path:
/// `NSApp.setApplicationIconImage:` needs an `NSImage`, and decoding
/// PNG bytes through Cocoa is the lightest route (no manual
/// `NSBitmapImageRep` assembly from raw RGBA). Loaders that don't
/// have the source bytes (raw-RGBA constructors, embedded resources)
/// leave it `None` and the macOS Dock falls back to the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIcon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    source_png: Option<Vec<u8>>,
}

/// Errors a host's icon pipeline can return — both decoder errors
/// from [`AppIconLoader::load`] and validation failures from
/// [`AppIcon::new`].
#[derive(Debug, Clone)]
pub enum IconError {
    /// The schema's `app.icon` source string was empty or pointed at
    /// a path / URL the loader couldn't read. The string carries the
    /// loader-supplied diagnostic.
    UnreadableSource(String),
    /// The decoded pixel buffer's length didn't match
    /// `width * height * 4`.
    SizeMismatch { width: u32, height: u32, got: usize },
    /// A backend-specific decoder error (PNG decode failure, ICO
    /// parse failure, network timeout for a remote URL, etc.). The
    /// string is loader-supplied; callers log it but don't typically
    /// parse it.
    Decode(String),
    /// The host hasn't wired a real loader. [`NullAppIconLoader`]
    /// returns this so callers can distinguish "no loader available"
    /// from "loader rejected this source".
    NoLoaderConfigured,
}

impl fmt::Display for IconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IconError::UnreadableSource(s) => write!(f, "icon source unreadable: {s}"),
            IconError::SizeMismatch { width, height, got } => {
                // Match `AppIcon::new`'s saturating math so a
                // pathologically-large dimension doesn't overflow
                // the format computation while we're trying to
                // *report* an overflow-shaped error.
                let expected = (*width as usize)
                    .saturating_mul(*height as usize)
                    .saturating_mul(4);
                write!(
                    f,
                    "icon pixel buffer size mismatch: expected {width}*{height}*4 = {expected} bytes, got {got}"
                )
            }
            IconError::Decode(s) => write!(f, "icon decode error: {s}"),
            IconError::NoLoaderConfigured => {
                f.write_str("no app-icon loader configured on this host")
            }
        }
    }
}

impl std::error::Error for IconError {}

impl AppIcon {
    /// Build an icon from already-decoded RGBA8 pixel data. Returns
    /// [`IconError::SizeMismatch`] if `rgba.len()` doesn't match
    /// `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, IconError> {
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if rgba.len() != expected {
            return Err(IconError::SizeMismatch {
                width,
                height,
                got: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
            source_png: None,
        })
    }

    /// Attach the original PNG bytes the icon was decoded from. Used
    /// by the macOS Dock helper — see [`set_macos_dock_icon_from_png`]
    /// — so callers don't have to track them separately. The bytes
    /// are stored as-is; no re-encoding.
    pub fn with_source_png(mut self, png_bytes: Vec<u8>) -> Self {
        self.source_png = Some(png_bytes);
        self
    }

    /// Borrow the original PNG bytes if the loader recorded them.
    /// Returns `None` for icons constructed from raw RGBA without a
    /// known source.
    pub fn source_png(&self) -> Option<&[u8]> {
        self.source_png.as_deref()
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Borrow the RGBA8 pixel buffer. Length is `width * height * 4`.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Consume the icon and return the underlying `(width, height,
    /// rgba)` triple — useful when handing the buffer to
    /// `winit::window::Icon::from_rgba` which takes ownership.
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba)
    }
}

/// Loads an [`AppIcon`] from the schema's `app.icon` source string.
///
/// The trait is deliberately minimal so each host can plug in
/// whatever decoder it ships (most platforms decode PNG; macOS hosts
/// might prefer `.icns`; web hosts decode data URIs). The schema
/// doesn't mandate a wire format — the loader is responsible for
/// recognising what `source` is.
///
/// ### Threading & sharing
///
/// `load` takes `&self` so stateless loaders (the common case — the
/// loader is a thin wrapper over `image::open`) need no interior
/// mutability. Stateful loaders (caching, bundled-asset lookup
/// tables) use `RefCell` for main-thread-only or `Mutex` for shared
/// access. The trait does not require `Send + Sync`: icon loading
/// runs once at window-creation time on the main thread. Hosts
/// wanting cross-thread sharing add the bound at their use site
/// (`Arc<dyn AppIconLoader + Send + Sync>`).
pub trait AppIconLoader {
    fn load(&self, source: &str) -> Result<AppIcon, IconError>;
}

/// No-op default. Reports [`IconError::NoLoaderConfigured`] for any
/// source. Hosts without a real loader wired up use this so the
/// `AppIconLoader` trait surface stays uniform.
#[derive(Debug, Default, Copy, Clone)]
pub struct NullAppIconLoader;

impl AppIconLoader for NullAppIconLoader {
    fn load(&self, _source: &str) -> Result<AppIcon, IconError> {
        Err(IconError::NoLoaderConfigured)
    }
}

/// Convert an [`AppIcon`] into a `winit::window::Icon` ready for
/// `WindowAttributes::with_window_icon`. Only available with the
/// `run` feature because winit is the run-loop crate's dependency.
#[cfg(feature = "run")]
pub fn to_winit_icon(icon: AppIcon) -> Result<winit::window::Icon, IconError> {
    let (w, h, rgba) = icon.into_parts();
    winit::window::Icon::from_rgba(rgba, w, h)
        .map_err(|e| IconError::Decode(format!("winit::Icon::from_rgba: {e}")))
}

/// macOS-only: set the running app's Dock icon from raw PNG bytes.
///
/// On macOS, an unbundled binary (`cargo run` style) shows the default
/// terminal/exec icon in the Dock because the Dock icon comes from the
/// `.app` bundle's `Contents/Resources/<name>.icns`. winit's
/// `set_window_icon` is a no-op on macOS — the only runtime hook is
/// `NSApp.setApplicationIconImage:`. This helper builds an `NSImage`
/// from the PNG bytes and applies it.
///
/// Must be called on the main thread. Safe to call before the event
/// loop starts: `NSApplication.sharedApplication` lazy-initialises
/// the global app object, and the icon assignment persists.
///
/// `png_bytes` is the raw `.png` file contents. The decoded
/// [`AppIcon`] RGBA buffer would also work but requires assembling
/// an `NSBitmapImageRep` by hand; PNG bytes route through Cocoa's
/// own decoder which is simpler and supports any color space the
/// PNG declares (the in-process `png` crate strips down to RGBA8).
#[cfg(target_os = "macos")]
pub fn set_macos_dock_icon_from_png(png_bytes: &[u8]) -> Result<(), String> {
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "set_macos_dock_icon must be called on the main thread".to_string())?;

    // `with_bytes` copies the slice into an immutable NSData. The
    // slice is borrowed for the duration of the call only.
    let data = NSData::with_bytes(png_bytes);

    // `initWithData:` is the documented PNG-decoding entry point.
    // Returns nil if the data isn't a recognised image format.
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "NSImage::initWithData returned nil — PNG decode failed".to_string())?;

    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: `setApplicationIconImage:` is marked unsafe in
    // objc2-app-kit because it accesses the global app object's
    // mutable state — but we have a `MainThreadMarker`, which is
    // exactly the invariant the marker exists to encode (Cocoa
    // requires NSApp mutations on the main thread). The image is
    // retained by Cocoa across the call; the local `Retained<NSImage>`
    // drops at end of scope, leaving Cocoa with its own retain count.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    Ok(())
}

/// No-op stub on non-macOS so callers don't need `cfg` everywhere.
/// Other platforms apply their icon via `winit::Window::set_window_icon`
/// (Windows / X11) or the launcher's `.desktop` reference (Wayland).
#[cfg(not(target_os = "macos"))]
pub fn set_macos_dock_icon_from_png(_png_bytes: &[u8]) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_for(width: u32, height: u32, fill: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            out.extend_from_slice(&fill);
        }
        out
    }

    #[test]
    fn new_validates_correct_size() {
        let icon = AppIcon::new(16, 16, rgba_for(16, 16, [0, 0, 0, 255])).unwrap();
        assert_eq!(icon.width(), 16);
        assert_eq!(icon.height(), 16);
        assert_eq!(icon.rgba().len(), 16 * 16 * 4);
    }

    #[test]
    fn new_rejects_short_buffer() {
        let too_short = vec![0u8; 16 * 16 * 4 - 1];
        match AppIcon::new(16, 16, too_short) {
            Err(IconError::SizeMismatch {
                width: 16,
                height: 16,
                got,
            }) => assert_eq!(got, 16 * 16 * 4 - 1),
            other => panic!("expected SizeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn new_rejects_long_buffer() {
        let too_long = vec![0u8; 16 * 16 * 4 + 4];
        assert!(AppIcon::new(16, 16, too_long).is_err());
    }

    #[test]
    fn into_parts_round_trips() {
        let original = rgba_for(8, 8, [10, 20, 30, 255]);
        let icon = AppIcon::new(8, 8, original.clone()).unwrap();
        let (w, h, rgba) = icon.into_parts();
        assert_eq!((w, h), (8, 8));
        assert_eq!(rgba, original);
    }

    #[test]
    fn null_loader_returns_no_loader_configured() {
        let l = NullAppIconLoader;
        match l.load("/path/to/icon.png") {
            Err(IconError::NoLoaderConfigured) => {}
            other => panic!("expected NoLoaderConfigured, got {other:?}"),
        }
    }

    #[test]
    fn icon_error_display_strings_are_useful() {
        assert!(IconError::UnreadableSource("file not found".into())
            .to_string()
            .contains("file not found"));
        assert!(IconError::Decode("bad PNG header".into())
            .to_string()
            .contains("bad PNG header"));
        let mismatch = IconError::SizeMismatch {
            width: 16,
            height: 16,
            got: 100,
        };
        let s = mismatch.to_string();
        assert!(s.contains("16*16*4 = 1024") && s.contains("got 100"));
    }

    /// Codex round 1 MINOR Q7: cfg-gated `to_winit_icon` wrapper had
    /// no test coverage. Build a 4×4 fully-opaque red icon, convert,
    /// and verify the resulting `winit::window::Icon` is a struct
    /// (the type is opaque, but successful construction is the
    /// useful invariant — `Icon::from_rgba` validates the
    /// dimension/length contract internally).
    #[cfg(feature = "run")]
    #[test]
    fn to_winit_icon_succeeds_for_valid_rgba() {
        let icon = AppIcon::new(4, 4, rgba_for(4, 4, [0xff, 0x00, 0x00, 0xff])).unwrap();
        let _winit_icon = to_winit_icon(icon).expect("4x4 RGBA accepts");
    }

    /// Demonstrates the canonical custom-loader shape that real
    /// platform decoders (image::open, embedded asset lookup, data
    /// URI parser) follow: take the source string, return RGBA bytes
    /// or an `IconError`.
    #[test]
    fn custom_loader_decodes_inline_test_format() {
        // Toy "loader" that recognises the literal source string
        // "16x16:black" and serves a 16×16 black icon. Real loaders
        // dispatch by file extension / scheme / magic bytes.
        struct Stub;
        impl AppIconLoader for Stub {
            fn load(&self, source: &str) -> Result<AppIcon, IconError> {
                if source == "16x16:black" {
                    AppIcon::new(16, 16, vec![0u8; 16 * 16 * 4])
                } else {
                    Err(IconError::UnreadableSource(source.to_owned()))
                }
            }
        }
        let l = Stub;
        let icon = l.load("16x16:black").expect("stub serves canned icon");
        assert_eq!(icon.width(), 16);
        assert_eq!(icon.height(), 16);
        assert!(matches!(
            l.load("missing.png"),
            Err(IconError::UnreadableSource(_))
        ));
    }
}
