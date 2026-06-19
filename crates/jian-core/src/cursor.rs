//! Cursor affordance vocabulary — a platform-free hint a widget or host can
//! return for a given screen point. Hosts map each variant to their platform
//! cursor (`winit::CursorIcon` on desktop, a CSS `cursor:` string on web). The
//! enum carries no platform types, so it is wasm32-clean and shared by
//! `jian-widgets` components (via `cursor_at`) and host code alike.

/// Cursor affordance suggested for a screen point. Widgets return this from
/// `cursor_at`; hosts own the final mapping to a platform cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorHint {
    /// Default arrow.
    Default,
    /// Pointer / hand — over a clickable affordance (shadcn `cursor-pointer`).
    Pointer,
    /// Move / drag affordance.
    Move,
    /// Closed hand — a grab/pan drag is in progress.
    Grabbing,
    /// Open hand — Hand tool over canvas, ready to grab + pan.
    Grab,
    /// Crosshair — shape / frame / text tool ready to draw on canvas. Distinct
    /// from `Default` so the click-spawns-a-node intent reads clearly.
    Crosshair,
    /// Text I-beam — text editing / Text tool over canvas.
    Text,
    /// Not-allowed — over a disabled affordance (CSS `not-allowed`).
    NotAllowed,
    /// East-west resize (left / right edges).
    ResizeEw,
    /// North-south resize (top / bottom edges).
    ResizeNs,
    /// NW-SE diagonal resize (top-left / bottom-right corners).
    ResizeNwse,
    /// NE-SW diagonal resize (top-right / bottom-left corners).
    ResizeNesw,
    /// Rotation affordance (corner rotate ring).
    Rotate,
}
