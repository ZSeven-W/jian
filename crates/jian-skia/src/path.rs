//! `PathCommand` → `skia_safe::Path`.

use crate::convert::to_sk_point;
use jian_core::render::PathCommand;
use skia_safe::Path as SkPath;

pub fn to_sk_path(commands: &[PathCommand]) -> SkPath {
    let mut path = SkPath::new();
    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo(p) => {
                path.move_to(to_sk_point(p));
            }
            PathCommand::LineTo(p) => {
                path.line_to(to_sk_point(p));
            }
            PathCommand::QuadTo(c, p) => {
                path.quad_to(to_sk_point(c), to_sk_point(p));
            }
            PathCommand::CubicTo(c1, c2, p) => {
                path.cubic_to(to_sk_point(c1), to_sk_point(c2), to_sk_point(p));
            }
            PathCommand::Close => {
                path.close();
            }
        }
    }
    path
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
