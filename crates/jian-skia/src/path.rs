//! `PathCommand` → `skia_safe::Path`.
//!
//! skia-safe 0.97 split path construction onto `PathBuilder` (mutating
//! `move_to` / `line_to` / `quad_to` / `cubic_to` / `close`); the
//! finished `Path` is produced by `builder.detach()` and is itself
//! immutable for traversal. See `~/.cargo/registry/src/.../skia-safe-0.97.0/src/core/path_builder.rs:238-360`.

use crate::convert::to_sk_point;
use jian_core::render::PathCommand;
use skia_safe::{Path as SkPath, PathBuilder};

pub fn to_sk_path(commands: &[PathCommand]) -> SkPath {
    let mut builder = PathBuilder::new();
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(p) => {
                builder.move_to(to_sk_point(p));
            }
            PathCommand::LineTo(p) => {
                builder.line_to(to_sk_point(p));
            }
            PathCommand::QuadTo(c, p) => {
                builder.quad_to(to_sk_point(c), to_sk_point(p));
            }
            PathCommand::CubicTo(c1, c2, p) => {
                builder.cubic_to(to_sk_point(c1), to_sk_point(c2), to_sk_point(p));
            }
            PathCommand::Close => {
                builder.close();
            }
        }
    }
    builder.detach()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jian_core::geometry::point;

    #[test]
    fn triangle_has_nonempty_bounds() {
        let cmds = [
            PathCommand::MoveTo(point(0.0, 0.0)),
            PathCommand::LineTo(point(10.0, 0.0)),
            PathCommand::LineTo(point(5.0, 10.0)),
            PathCommand::Close,
        ];
        let path = to_sk_path(&cmds);
        let bounds = path.compute_tight_bounds();
        assert!(bounds.width() > 0.0);
        assert!(bounds.height() > 0.0);
    }

    #[test]
    fn empty_commands_yield_empty_path() {
        let path = to_sk_path(&[]);
        assert!(path.is_empty());
    }
}
