# Changelog

## [0.1.0] — Unreleased

### Added

- Runtime composition root (`Runtime`).
- Document runtime (SlotMap-backed tree + ID index).
- Fine-grained reactive primitives: `Signal<T>`, `Scheduler`, `Effect`.
- State graph with six scopes: `$app`, `$page`, `$self`, `$route`, `$storage`, `$vars`.
- Layout engine via `taffy` 0.5 (basic flexbox mapping).
- Spatial index via `rstar` (hit + rect queries).
- Viewport math with screen↔scene transforms.
- `RenderBackend` trait + `CaptureBackend` for dry-run / tests.
- `LogicProvider` trait (Tier 3, L4 reserved).
- End-to-end pipeline smoke test (`counter.op` fixture).
- Signal update microbenchmark (10/100/1000 subscribers).
