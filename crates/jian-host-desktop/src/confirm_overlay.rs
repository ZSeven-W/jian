use jian_core::geometry::{point, rect};
use jian_core::render::{DrawOp, Paint, TextAlign, TextRun};
use jian_core::scene::Color;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

#[derive(Clone, Default)]
pub struct ConfirmOverlay(Rc<RefCell<State>>);

#[derive(Default)]
struct State {
    next_id: u64,
    active: Option<Dialog>,
}

struct Dialog {
    id: u64,
    title: String,
    message: String,
    result: Option<bool>,
    waker: Option<Waker>,
}

pub struct ConfirmFuture {
    overlay: ConfirmOverlay,
    id: u64,
}

#[derive(Clone)]
pub struct DismissHandle {
    overlay: ConfirmOverlay,
    id: u64,
}

impl ConfirmOverlay {
    pub fn present(
        &self,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> (ConfirmFuture, DismissHandle) {
        self.dismiss_active();
        let mut state = self.0.borrow_mut();
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let id = state.next_id;
        state.active = Some(Dialog {
            id,
            title: title.into(),
            message: message.into(),
            result: None,
            waker: None,
        });
        (
            ConfirmFuture {
                overlay: self.clone(),
                id,
            },
            DismissHandle {
                overlay: self.clone(),
                id,
            },
        )
    }

    pub fn is_active(&self) -> bool {
        self.0.borrow().active.is_some()
    }
    pub fn confirm(&self) {
        self.resolve_active(true);
    }
    pub fn cancel(&self) {
        self.resolve_active(false);
    }
    pub fn handle_key(&self, key: &str) -> bool {
        match key {
            "Enter" => self.confirm(),
            "Escape" => self.cancel(),
            _ => return false,
        }
        true
    }
    pub fn handle_click(&self, x: f32, y: f32, width: f32, height: f32) -> bool {
        if !self.is_active() {
            return false;
        }
        let panel_w = width.clamp(280.0, 480.0);
        let panel_h = 220.0;
        let left = (width - panel_w) / 2.0;
        let top = (height - panel_h) / 2.0;
        if y >= top + panel_h - 56.0 && y <= top + panel_h - 22.0 {
            if x >= left + panel_w - 104.0 && x <= left + panel_w - 24.0 {
                self.confirm();
                return true;
            }
            if x >= left + panel_w - 196.0 && x <= left + panel_w - 120.0 {
                self.cancel();
                return true;
            }
        }
        true
    }
    fn resolve_active(&self, value: bool) {
        let mut state = self.0.borrow_mut();
        if let Some(dialog) = state.active.as_mut() {
            dialog.result = Some(value);
            if let Some(waker) = dialog.waker.take() {
                waker.wake();
            }
        }
    }
    fn dismiss(&self, id: u64) {
        let active = self
            .0
            .borrow()
            .active
            .as_ref()
            .is_some_and(|dialog| dialog.id == id);
        if active {
            self.resolve_active(false);
        }
    }
    fn dismiss_active(&self) {
        let id = self.0.borrow().active.as_ref().map(|dialog| dialog.id);
        if let Some(id) = id {
            self.dismiss(id);
        }
    }
    pub fn draw_ops(&self, width: f32, height: f32) -> Vec<DrawOp> {
        let state = self.0.borrow();
        let Some(dialog) = state.active.as_ref() else {
            return Vec::new();
        };
        let panel_w = width.clamp(280.0, 480.0);
        let panel_h = 220.0;
        let x = (width - panel_w) / 2.0;
        let y = (height - panel_h) / 2.0;
        vec![
            DrawOp::Rect {
                rect: rect(0.0, 0.0, width, height),
                paint: Paint::solid(Color::rgba(0, 0, 0, 0x88)),
            },
            DrawOp::Rect {
                rect: rect(x, y, panel_w, panel_h),
                paint: Paint::solid(Color::rgb(0x24, 0x24, 0x27)),
            },
            DrawOp::Text(TextRun {
                content: dialog.title.clone(),
                font_family: "system-ui".into(),
                font_size: 20.0,
                font_weight: 700,
                color: Color::rgb(0xff, 0xff, 0xff),
                origin: point(x + 24.0, y + 24.0),
                max_width: panel_w - 48.0,
                align: TextAlign::Start,
                line_height: 1.2,
            }),
            DrawOp::Text(TextRun {
                content: dialog.message.clone(),
                font_family: "system-ui".into(),
                font_size: 14.0,
                font_weight: 400,
                color: Color::rgb(0xd4, 0xd4, 0xd8),
                origin: point(x + 24.0, y + 72.0),
                max_width: panel_w - 48.0,
                align: TextAlign::Start,
                line_height: 1.4,
            }),
            DrawOp::Rect {
                rect: rect(x + panel_w - 196.0, y + panel_h - 56.0, 76.0, 34.0),
                paint: Paint::solid(Color::rgb(0x3f, 0x3f, 0x46)),
            },
            DrawOp::Text(TextRun {
                content: "Cancel".into(),
                font_family: "system-ui".into(),
                font_size: 14.0,
                font_weight: 500,
                color: Color::rgb(0xff, 0xff, 0xff),
                origin: point(x + panel_w - 196.0, y + panel_h - 48.0),
                max_width: 76.0,
                align: TextAlign::Center,
                line_height: 1.0,
            }),
            DrawOp::Rect {
                rect: rect(x + panel_w - 104.0, y + panel_h - 56.0, 80.0, 34.0),
                paint: Paint::solid(Color::rgb(0x25, 0x63, 0xeb)),
            },
            DrawOp::Text(TextRun {
                content: "OK".into(),
                font_family: "system-ui".into(),
                font_size: 14.0,
                font_weight: 600,
                color: Color::rgb(0xff, 0xff, 0xff),
                origin: point(x + panel_w - 104.0, y + panel_h - 48.0),
                max_width: 80.0,
                align: TextAlign::Center,
                line_height: 1.0,
            }),
        ]
    }
}

impl DismissHandle {
    pub fn dismiss(&self) {
        self.overlay.dismiss(self.id);
    }
}
impl Drop for ConfirmFuture {
    fn drop(&mut self) {
        self.overlay.dismiss(self.id);
    }
}
impl Future for ConfirmFuture {
    type Output = bool;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        let mut state = self.overlay.0.borrow_mut();
        let Some(dialog) = state.active.as_mut().filter(|dialog| dialog.id == self.id) else {
            return Poll::Ready(false);
        };
        if let Some(value) = dialog.result.take() {
            state.active = None;
            Poll::Ready(value)
        } else {
            dialog.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll};
    #[test]
    fn enter_escape_and_dismiss_resolve_without_native_dialog() {
        let overlay = ConfirmOverlay::default();
        let (mut future, _) = overlay.present("Delete?", "This cannot be undone");
        let waker = futures::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        assert!(matches!(Pin::new(&mut future).poll(&mut context), Poll::Pending));
        assert!(overlay.handle_key("Enter"));
        assert!(futures::executor::block_on(future));
        let (future, dismiss) = overlay.present("Delete?", "Again");
        dismiss.dismiss();
        assert!(!futures::executor::block_on(future));
        assert!(!overlay.is_active());
    }
}
