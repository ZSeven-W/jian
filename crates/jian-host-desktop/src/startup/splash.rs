//! Splash-frame renderer + minimum-duration timer (Plan 19 Task 7).
//!
//! [`SplashRenderer`] paints a single splash frame from a
//! [`jian_ops_schema::app::SplashConfig`]. [`SplashTimer`] tracks the
//! configured `min_duration_ms` so the host knows when it's safe to
//! cross-fade out, even if the real first frame becomes ready earlier
//! (without the timer the splash would flash for sub-frame durations
//! and look broken on fast machines).
//!
//! ## Why these two pieces are split
//!
//! Plan 19 §C19 wants splash → real-content transitions to be
//! cross-faded so users don't see a hard cut. The cross-fade owner is
//! the run loop (it has access to the per-frame schedule + opacity
//! animation hooks). This module provides the **inputs** the cross-fade
//! needs:
//!
//! - "Is the splash on screen yet?" — `SplashRenderer::render` returns
//!   the `Instant` it started; pass that into `SplashTimer::new`.
//! - "Has the configured minimum duration elapsed?" — `SplashTimer::is_elapsed`.
//! - "How long until it elapses?" — `SplashTimer::remaining` so the
//!   run loop can `request_redraw_after(remaining)` rather than busy-poll.
//!
//! The actual cross-fade animation is intentionally out of scope: it
//! belongs in the winit run loop where opacity per-frame interpolation
//! sits next to redraw-scheduling.
//!
//! ## What the renderer paints
//!
//! For Task 7 the renderer paints a single solid-color background fill
//! across the whole canvas. Optional fields on `SplashConfig`
//! (`image`, `text`) are not yet rendered — they require image-asset
//! loading + text shaping that overlap with Plan 19 Task 4 (font
//! subsetting) and Plan 8 (asset bundling). Those land in a follow-up;
//! the background-only splash is enough to honour the spec's "user
//! sees something during 0-300 ms" guarantee on every platform.

#[cfg(test)]
use jian_core::geometry::size;
use jian_core::geometry::{rect, Size};
use jian_core::render::{DrawOp, Paint, RenderBackend};
use jian_core::scene::Color;
use jian_ops_schema::app::SplashConfig;
use std::time::{Duration, Instant};

/// Default minimum on-screen duration when `SplashConfig::min_duration_ms`
/// is `None`. Matches the spec example (200 ms) — long enough for the
/// human eye to register the splash, short enough that fast machines
/// don't get held back unnecessarily.
pub const DEFAULT_MIN_DURATION_MS: u32 = 200;

/// Fallback background when `SplashConfig::background` is missing or
/// not a parseable hex string. Matches the spec example (`#0a0a0a` —
/// near-black). Chosen to be visible on top of any uninitialised
/// surface buffer (most graphics drivers clear to opaque black, so
/// near-black on black is invisible to the user but still semantically
/// "the splash is up").
pub const DEFAULT_BACKGROUND: Color = Color::rgb(0x0a, 0x0a, 0x0a);

/// Paints a splash frame from a [`SplashConfig`].
///
/// Construct with [`SplashRenderer::new`]; call [`SplashRenderer::render`]
/// once per cold start (re-rendering the splash after the real frame
/// is up is an animation concern, owned by the run loop's cross-fade).
pub struct SplashRenderer {
    config: SplashConfig,
}

impl SplashRenderer {
    pub fn new(config: SplashConfig) -> Self {
        Self { config }
    }

    /// Borrow the underlying config — useful when the run loop wants
    /// to read `min_duration_ms` to build a [`SplashTimer`] without
    /// cloning the renderer.
    pub fn config(&self) -> &SplashConfig {
        &self.config
    }

    /// Paint the splash onto `surface` via `backend`. Returns the
    /// [`Instant`] the render started so the caller can pair it with
    /// [`SplashTimer::new`] without re-querying the clock.
    ///
    /// `canvas_size` is the host-supplied logical size of `surface` —
    /// the renderer fills the whole rect with the background color so
    /// the splash covers the entire window regardless of OS-specific
    /// surface-size accounting. (`SkiaSurface::width()`/`height()` are
    /// available on the real path; tests pass an explicit size.)
    ///
    /// **The returned `Instant` is render-start, not present-to-screen
    /// time.** On a single-buffered presenter the gap is negligible; on
    /// double-buffered or compositor-mediated presenters there can be
    /// up to one refresh interval of latency before the user actually
    /// sees the splash. Cross-fade code that needs sub-frame accuracy
    /// should subtract a configurable `present_offset` rather than
    /// treat this as wall-clock truth.
    #[must_use = "the returned Instant is the timer's t=0; pair it with \
                  SplashTimer::new (or use paint_with_timer)"]
    pub fn render<B: RenderBackend>(
        &self,
        backend: &mut B,
        surface: &mut B::Surface,
        canvas_size: Size,
    ) -> Instant {
        let started = Instant::now();
        let bg = self.background_color();
        // begin_frame's `clear` is ARGB-encoded in jian-skia and ignored
        // by CaptureBackend; passing 0 (fully-transparent) is safe in
        // both because we immediately overdraw the whole canvas with
        // the splash background. Avoids the RGBA→ARGB conversion the
        // RenderBackend trait doesn't currently codify.
        backend.begin_frame(surface, 0);
        backend.draw(&DrawOp::Rect {
            rect: rect(0.0, 0.0, canvas_size.width, canvas_size.height),
            paint: Paint::solid(bg),
        });
        backend.end_frame(surface);
        started
    }

    /// Resolved background color: parses `config.background` as a hex
    /// string and falls back to [`DEFAULT_BACKGROUND`] on failure.
    pub fn background_color(&self) -> Color {
        self.config
            .background
            .as_deref()
            .and_then(Color::from_hex)
            .unwrap_or(DEFAULT_BACKGROUND)
    }
}

/// Tracks the splash's minimum on-screen duration.
///
/// Built with [`SplashTimer::new`] from the `Instant` returned by
/// [`SplashRenderer::render`] plus the splash config. The run loop
/// polls [`SplashTimer::is_elapsed`] each frame; once it returns
/// `true` AND the real first frame is ready, cross-fade in.
pub struct SplashTimer {
    rendered_at: Instant,
    min_duration: Duration,
}

impl SplashTimer {
    pub fn new(rendered_at: Instant, config: &SplashConfig) -> Self {
        let min_ms = config.min_duration_ms.unwrap_or(DEFAULT_MIN_DURATION_MS);
        Self {
            rendered_at,
            min_duration: Duration::from_millis(u64::from(min_ms)),
        }
    }

    /// `true` once the configured minimum on-screen duration has passed.
    pub fn is_elapsed(&self) -> bool {
        self.rendered_at.elapsed() >= self.min_duration
    }

    /// Time remaining until [`Self::is_elapsed`] flips to `true`.
    /// Saturates to zero — the run loop can pass this directly to
    /// `request_redraw_after`.
    pub fn remaining(&self) -> Duration {
        self.min_duration.saturating_sub(self.rendered_at.elapsed())
    }

    /// The configured minimum duration (independent of when render
    /// started). Useful for tracing.
    pub fn min_duration(&self) -> Duration {
        self.min_duration
    }
}

/// Convenience constructor: render the splash AND build the timer in
/// one call, returning both. Removes the boilerplate of threading the
/// returned `Instant` from `render` into `SplashTimer::new`.
pub fn paint_with_timer<B: RenderBackend>(
    config: SplashConfig,
    backend: &mut B,
    surface: &mut B::Surface,
    canvas_size: Size,
) -> (SplashRenderer, SplashTimer) {
    let renderer = SplashRenderer::new(config);
    let started = renderer.render(backend, surface, canvas_size);
    let timer = SplashTimer::new(started, renderer.config());
    (renderer, timer)
}

/// Same as [`paint_with_timer`] but takes a default size hint when the
/// caller doesn't have a real surface yet — useful in headless tests.
#[cfg(test)]
fn fixture_size() -> Size {
    size(800.0, 600.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::render::{CaptureBackend, RenderCommand};
    use jian_ops_schema::app::SplashConfig;
    use std::thread;

    fn cfg(background: Option<&str>, min_ms: Option<u32>) -> SplashConfig {
        SplashConfig {
            background: background.map(str::to_owned),
            image: None,
            text: None,
            min_duration_ms: min_ms,
        }
    }

    #[test]
    fn background_color_parses_hex_string() {
        let r = SplashRenderer::new(cfg(Some("#102a43"), None));
        let c = r.background_color();
        assert_eq!(c.r(), 0x10);
        assert_eq!(c.g(), 0x2a);
        assert_eq!(c.b(), 0x43);
    }

    #[test]
    fn background_color_falls_back_when_missing() {
        let r = SplashRenderer::new(cfg(None, None));
        assert_eq!(r.background_color(), DEFAULT_BACKGROUND);
    }

    #[test]
    fn background_color_falls_back_when_not_parseable() {
        let r = SplashRenderer::new(cfg(Some("not-a-color"), None));
        assert_eq!(r.background_color(), DEFAULT_BACKGROUND);
    }

    #[test]
    fn render_emits_begin_full_canvas_rect_end() {
        let mut backend = CaptureBackend::new();
        let mut surface = backend.new_surface(fixture_size());
        let r = SplashRenderer::new(cfg(Some("#0a0a0a"), None));
        let _started = r.render(&mut backend, &mut surface, fixture_size());
        let cmds = backend.take();
        assert_eq!(cmds.len(), 3, "begin_frame + 1 rect + end_frame: {cmds:?}");
        match &cmds[0] {
            RenderCommand::BeginFrame { clear: 0 } => {}
            other => panic!("expected BeginFrame {{ clear: 0 }}, got {other:?}"),
        }
        match &cmds[1] {
            RenderCommand::Draw(DrawOp::Rect { rect: r, paint }) => {
                assert_eq!(r.size.width, fixture_size().width);
                assert_eq!(r.size.height, fixture_size().height);
                assert_eq!(paint.fill, Some(Color::rgb(0x0a, 0x0a, 0x0a)));
                assert_eq!(paint.opacity, 1.0);
                assert!(paint.stroke.is_none());
            }
            other => panic!("expected full-canvas solid Rect, got {other:?}"),
        }
        assert!(matches!(cmds[2], RenderCommand::EndFrame));
    }

    #[test]
    fn render_returns_start_instant() {
        let mut backend = CaptureBackend::new();
        let mut surface = backend.new_surface(fixture_size());
        let r = SplashRenderer::new(cfg(None, None));
        let before = Instant::now();
        let started = r.render(&mut backend, &mut surface, fixture_size());
        let after = Instant::now();
        assert!(
            started >= before && started <= after,
            "render's reported Instant ({started:?}) must be in [{before:?}, {after:?}]"
        );
    }

    #[test]
    fn timer_uses_default_duration_when_config_missing() {
        let now = Instant::now();
        let t = SplashTimer::new(now, &cfg(None, None));
        assert_eq!(
            t.min_duration(),
            Duration::from_millis(DEFAULT_MIN_DURATION_MS as u64)
        );
    }

    #[test]
    fn timer_uses_configured_duration() {
        let now = Instant::now();
        let t = SplashTimer::new(now, &cfg(None, Some(500)));
        assert_eq!(t.min_duration(), Duration::from_millis(500));
    }

    #[test]
    fn timer_is_elapsed_after_min_duration_passes() {
        let now = Instant::now();
        let t = SplashTimer::new(now, &cfg(None, Some(20)));
        assert!(!t.is_elapsed(), "fresh timer should not be elapsed");
        thread::sleep(Duration::from_millis(40));
        assert!(
            t.is_elapsed(),
            "timer should be elapsed after 2x min duration"
        );
        assert_eq!(t.remaining(), Duration::ZERO);
    }

    #[test]
    fn timer_remaining_decreases_monotonically() {
        let t = SplashTimer::new(Instant::now(), &cfg(None, Some(100)));
        let r1 = t.remaining();
        thread::sleep(Duration::from_millis(20));
        let r2 = t.remaining();
        assert!(
            r2 < r1,
            "remaining must decrease as time passes: {r1:?} → {r2:?}"
        );
    }

    #[test]
    fn paint_with_timer_returns_consistent_pair() {
        let mut backend = CaptureBackend::new();
        let mut surface = backend.new_surface(fixture_size());
        let (renderer, timer) = paint_with_timer(
            cfg(Some("#1e88e5"), Some(150)),
            &mut backend,
            &mut surface,
            fixture_size(),
        );
        // Renderer + timer were built from the same config.
        assert_eq!(renderer.background_color(), Color::rgb(0x1e, 0x88, 0xe5));
        assert_eq!(timer.min_duration(), Duration::from_millis(150));
        // Render path emitted begin + rect + end.
        assert_eq!(backend.take().len(), 3);
    }
}
