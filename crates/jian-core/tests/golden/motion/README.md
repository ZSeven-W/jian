# Motion golden vectors

Recorded from OpenPencil's original preview motion runtime before it moved
into `jian_core::motion`, so the moved code can be checked value for value.

- Generator: OpenPencil `crates/op-preview-core/src/tests_motion_golden.rs`
  (added in `c3c977a13`, in-view fixture driven through a phone frame since
  `807785320`).
- Recorded at: OpenPencil `v0.8.6@12fdd7736` with this jian at `f5a3362`.
  Two runs were byte-identical.
- Regenerate (only before changing behaviour on purpose):
  `OP_MOTION_GOLDEN_OUT=<this dir> cargo test -p op-preview-core tests_motion_golden -- --ignored`

| File | Content | Used by |
| --- | --- | --- |
| `curves.json` | every schema easing sampled at `i / 1000`, `i = 0..=1000` (f32) | jian-core `motion_golden.rs` |
| `tracks.json` | 30 single-track requests sampled every 8 ms | jian-core `motion_golden.rs` |
| `transitions.json` | push / pop / replace progress every 8 ms | jian-core `motion_golden.rs` |
| `sessions.json` | three whole preview sessions every 16 ms | OpenPencil session parity (needs its scene loader) |

The vectors keep today's quirks: curve x is solved with 16 bisection steps,
so `standard` reads 1.7e-10 at p = 0 and `emphasizedAccelerate` reaches only
0.99998 at p = 1.
