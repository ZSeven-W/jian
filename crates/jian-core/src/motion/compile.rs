//! Compile the runtime's motion curves into per-segment cubic-beziers.
//!
//! The runtime eases a whole iteration once and then interpolates keyframes
//! linearly in eased progress: `value(u) = L(clamp(E(u)))`. CSS, Lottie and
//! SwiftUI instead apply a timing function per keyframe interval. They agree
//! once the iteration is cut wherever `E` crosses a keyframe offset, leaves
//! [0, 1], or turns around: inside each piece every property is the same
//! affine function of `E`, so one cubic-bezier (the matching slice of `E`,
//! normalized) times all of them. Clamped stretches become holds.

use super::curve::{as_cubic, bezier, CubicPiece};
use super::interpolate::{interpolate_kind, interpolate_stops};
use crate::action::animation_registry::{animatable_property_registry, AnimationInterpolate};
use crate::action::services::{AnimationProperty, Easing};
use jian_ops_schema::motion::{NodeAnimation, P1_MOTION_PROPERTIES};
use serde_json::Value;
use std::collections::BTreeMap;

/// Segments past this many fall back to sampled linear pieces.
pub const MAX_SEGMENTS: usize = 64;
const DEGRADED_SEGMENTS: usize = 60;
const SCAN_STEPS: usize = 2048;
const LINEAR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A cubic's control polygon, endpoints included.
type Polygon = [(f32, f32); 4];

/// One keyframe interval of a compiled animation: on iteration time
/// `t0..t1` (fractions of the duration) every property runs from `from` to
/// `to` along `cubic-bezier(bezier)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledSegment {
    pub t0: f32,
    pub t1: f32,
    pub from: BTreeMap<String, Value>,
    pub to: BTreeMap<String, Value>,
    pub bezier: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledAnimation {
    pub segments: Vec<CompiledSegment>,
    /// True when the exact cut needed more than [`MAX_SEGMENTS`] pieces and
    /// the output is sampled linear pieces instead.
    pub degraded: bool,
}

struct Track {
    name: String,
    stops: Vec<(f32, Value)>,
    kind: AnimationInterpolate,
}

/// Compile one animation's single iteration.
pub fn compile(declaration: &NodeAnimation) -> CompiledAnimation {
    let easing = Easing::from(&declaration.easing);
    let tracks = tracks(declaration);
    let pieces = as_cubic(easing);
    let mut levels: Vec<f32> = tracks
        .iter()
        .flat_map(|track| track.stops.iter().map(|(offset, _)| *offset))
        .collect();
    levels.extend([0.0, 1.0]);
    let cuts = cut_points(&pieces, &levels);
    let mut segments = Vec::new();
    for window in cuts.windows(2) {
        push_segments(&mut segments, &pieces, &tracks, window[0], window[1], 0);
    }
    if segments.len() > MAX_SEGMENTS {
        return CompiledAnimation {
            segments: sampled_segments(&tracks, &pieces),
            degraded: true,
        };
    }
    CompiledAnimation {
        segments,
        degraded: false,
    }
}

fn tracks(declaration: &NodeAnimation) -> Vec<Track> {
    let mut by_property: BTreeMap<String, Vec<(f32, Value)>> = BTreeMap::new();
    for keyframe in &declaration.keyframes {
        for (property, value) in &keyframe.values {
            by_property
                .entry(property.clone())
                .or_default()
                .push((keyframe.offset, value.clone()));
        }
    }
    by_property
        .into_iter()
        .filter(|(name, _)| P1_MOTION_PROPERTIES.contains(&name.as_str()))
        .filter_map(|(name, stops)| {
            let descriptor = animatable_property_registry().get(&name)?;
            let property = AnimationProperty::from_registered(&name, descriptor.apply);
            Some(Track {
                kind: interpolate_kind(&property),
                name,
                stops,
            })
        })
        .collect()
}

/// Every property's value at iteration time `u`, on the exact curve the
/// segments' beziers are cut from (clamped like the runtime). The runtime
/// solves a bezier's x with only 16 bisection steps, so it can differ by up
/// to 5e-4 in progress; the export should not copy that noise (it would
/// start a 16 px slide at 15.9997 px).
fn values_at(tracks: &[Track], pieces: &[CubicPiece], u: f32) -> BTreeMap<String, Value> {
    let progress = raw(pieces, u).clamp(0.0, 1.0);
    tracks
        .iter()
        .map(|track| {
            (
                track.name.clone(),
                interpolate_stops(&track.stops, track.kind, progress),
            )
        })
        .collect()
}

fn piece_at(pieces: &[CubicPiece], u: f32) -> &CubicPiece {
    pieces
        .iter()
        .find(|piece| u <= piece.x1)
        .unwrap_or_else(|| pieces.last().expect("a curve has pieces"))
}

/// The piece-local bezier parameter whose x lands on `u`.
fn parameter_at(piece: &CubicPiece, u: f32) -> f32 {
    let span = (piece.x1 - piece.x0).max(f32::EPSILON);
    let local = ((u - piece.x0) / span).clamp(0.0, 1.0);
    // Bisection never lands on the ends exactly; a piece's ends are exact.
    if local <= 0.0 || local >= 1.0 {
        return local;
    }
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..40 {
        let t = (low + high) / 2.0;
        if bezier(t, piece.control[0], piece.control[2]) < local {
            low = t;
        } else {
            high = t;
        }
    }
    (low + high) / 2.0
}

/// The exact, unclamped curve value at `u`.
fn raw(pieces: &[CubicPiece], u: f32) -> f32 {
    let piece = piece_at(pieces, u);
    let t = parameter_at(piece, u);
    piece.y0 + (piece.y1 - piece.y0) * bezier(t, piece.control[1], piece.control[3])
}

/// Every cut in [0, 1]: piece boundaries, level crossings (keyframe offsets
/// and the clamp edges), and turning points.
fn cut_points(pieces: &[CubicPiece], levels: &[f32]) -> Vec<f32> {
    let mut cuts = vec![0.0, 1.0];
    cuts.extend(pieces.iter().map(|piece| piece.x0));
    let samples: Vec<f32> = (0..=SCAN_STEPS)
        .map(|step| raw(pieces, step as f32 / SCAN_STEPS as f32))
        .collect();
    for step in 0..SCAN_STEPS {
        let (u0, u1) = (
            step as f32 / SCAN_STEPS as f32,
            (step + 1) as f32 / SCAN_STEPS as f32,
        );
        let (v0, v1) = (samples[step], samples[step + 1]);
        for &level in levels {
            if (v0 - level).abs() < 1e-7 {
                // The grid landed exactly on the level (linear curves do).
                cuts.push(u0);
            } else if (v0 - level) * (v1 - level) < 0.0 {
                cuts.push(refine_crossing(pieces, level, u0, u1, v0 < level));
            }
        }
        if step > 0 {
            let before = samples[step] - samples[step - 1];
            let after = v1 - v0;
            if before * after < 0.0 {
                cuts.push(u0);
            }
        }
    }
    cuts.sort_by(f32::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    cuts
}

fn refine_crossing(
    pieces: &[CubicPiece],
    level: f32,
    mut low: f32,
    mut high: f32,
    rising: bool,
) -> f32 {
    for _ in 0..32 {
        let middle = (low + high) / 2.0;
        if (raw(pieces, middle) < level) == rising {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

fn push_segments(
    out: &mut Vec<CompiledSegment>,
    pieces: &[CubicPiece],
    tracks: &[Track],
    u0: f32,
    u1: f32,
    depth: u32,
) {
    let segment = |bezier| CompiledSegment {
        t0: u0,
        t1: u1,
        from: values_at(tracks, pieces, u0),
        to: values_at(tracks, pieces, u1),
        bezier,
    };
    let middle = raw(pieces, (u0 + u1) / 2.0);
    if !(0.0..=1.0).contains(&middle) {
        // The runtime clamps here, so the values hold.
        out.push(segment(LINEAR));
        return;
    }
    match slice_bezier(pieces, u0, u1) {
        Some(bezier) => out.push(segment(bezier)),
        None if depth < 8 => {
            let half = (u0 + u1) / 2.0;
            push_segments(out, pieces, tracks, u0, half, depth + 1);
            push_segments(out, pieces, tracks, half, u1, depth + 1);
        }
        None => out.push(segment(LINEAR)),
    }
}

/// The slice of the curve between `u0` and `u1` as a normalized
/// cubic-bezier, or `None` when its x controls leave [0, 1] (split further).
fn slice_bezier(pieces: &[CubicPiece], u0: f32, u1: f32) -> Option<[f32; 4]> {
    let piece = piece_at(pieces, (u0 + u1) / 2.0);
    let (t0, t1) = (parameter_at(piece, u0), parameter_at(piece, u1));
    let [x1, y1, x2, y2] = piece.control;
    let points = subdivide([(0.0, 0.0), (x1, y1), (x2, y2), (1.0, 1.0)], t0, t1);
    let (dx, dy) = (points[3].0 - points[0].0, points[3].1 - points[0].1);
    if dx.abs() < 1e-9 {
        return Some(LINEAR);
    }
    if dy.abs() < 1e-9 {
        // A flat slice: the values do not move, any timing works.
        return Some(LINEAR);
    }
    let normalize =
        |point: (f32, f32)| ((point.0 - points[0].0) / dx, (point.1 - points[0].1) / dy);
    let (a, b) = (normalize(points[1]), normalize(points[2]));
    let inside = |x: f32| (-1e-4..=1.0 + 1e-4).contains(&x);
    if !inside(a.0) || !inside(b.0) {
        return None;
    }
    Some([a.0.clamp(0.0, 1.0), a.1, b.0.clamp(0.0, 1.0), b.1])
}

/// The control polygon of a cubic restricted to parameters `t0..t1`.
fn subdivide(points: Polygon, t0: f32, t1: f32) -> Polygon {
    let tail = split_at(points, t0).1;
    if t0 >= 1.0 {
        return tail;
    }
    let local = ((t1 - t0) / (1.0 - t0)).clamp(0.0, 1.0);
    split_at(tail, local).0
}

/// De Casteljau split of a cubic at `t` into its two halves.
fn split_at(p: Polygon, t: f32) -> (Polygon, Polygon) {
    let lerp = |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let (a, b, c) = (lerp(p[0], p[1]), lerp(p[1], p[2]), lerp(p[2], p[3]));
    let (d, e) = (lerp(a, b), lerp(b, c));
    let f = lerp(d, e);
    ([p[0], a, d, f], [f, e, c, p[3]])
}

fn sampled_segments(tracks: &[Track], pieces: &[CubicPiece]) -> Vec<CompiledSegment> {
    (0..DEGRADED_SEGMENTS)
        .map(|index| {
            let u0 = index as f32 / DEGRADED_SEGMENTS as f32;
            let u1 = (index + 1) as f32 / DEGRADED_SEGMENTS as f32;
            CompiledSegment {
                t0: u0,
                t1: u1,
                from: values_at(tracks, pieces, u0),
                to: values_at(tracks, pieces, u1),
                bezier: LINEAR,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "compile_tests.rs"]
mod tests;
