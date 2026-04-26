//! `StartupReport` — per-phase timing collection produced by [`StartupDriver`].
//!
//! The report captures **when** each phase started (offset from
//! `StartupDriver::run` entry) and **how long** it took. The driver decides
//! parallelism; this module just records the result and offers two queries
//! callers care about: critical-path total, and a pretty table render.
//!
//! [`StartupDriver`]: crate::startup::driver::StartupDriver

use crate::startup::phase::StartupPhase;
use std::fmt::Write as _;

/// Timing record for a single phase run.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseTiming {
    pub phase: StartupPhase,
    /// Wall-clock offset from `StartupDriver::run` entry, in milliseconds.
    pub started_at_ms: f64,
    pub duration_ms: f64,
    /// Cached copy of [`StartupPhase::is_critical`] at record time so a
    /// pretty-print of an old report doesn't change meaning if the static
    /// classification ever shifts.
    pub on_critical: bool,
    /// Free-form annotation a phase impl can attach (`"Metal"`, `"12 KB"`,
    /// `"28 nodes"`). Plan 19 Task 8's `jian perf startup` table renders
    /// this column when at least one phase carries notes.
    pub notes: Option<String>,
}

impl PhaseTiming {
    pub fn ended_at_ms(&self) -> f64 {
        self.started_at_ms + self.duration_ms
    }
}

/// Collected timings for one cold-start run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StartupReport {
    pub phases: Vec<PhaseTiming>,
    /// Time from driver entry to `EventPumpReady` end, in milliseconds.
    /// Filled in by `StartupDriver::run` (in the sibling
    /// [`crate::startup::driver`] module) once the terminal phase
    /// completes; stays `0.0` if the run aborted before reaching
    /// `EventPumpReady`.
    pub first_interactive_ms: f64,
}

impl StartupReport {
    /// Sum of durations of phases marked `on_critical`.
    ///
    /// This is **not** the same as wall-clock-to-EventPumpReady when phases
    /// run in parallel — the latter is `first_interactive_ms`. Callers
    /// chasing the spec C19 budget should compare wall-clock, not this
    /// number; this number answers "if the critical path ran serially, how
    /// long would it take?".
    pub fn critical_path_ms(&self) -> f64 {
        self.phases
            .iter()
            .filter(|t| t.on_critical)
            .map(|t| t.duration_ms)
            .sum()
    }

    /// Wall-clock total: max `ended_at_ms` across all phases.
    pub fn total_wall_clock_ms(&self) -> f64 {
        self.phases
            .iter()
            .map(|t| t.ended_at_ms())
            .fold(0.0_f64, f64::max)
    }

    /// Human-readable table — used by `jian perf startup` (Plan 19 Task 8) and
    /// in test snapshots. The Notes column only renders when at least one
    /// phase has notes attached.
    pub fn pretty(&self) -> String {
        // Column widths chosen so 18-character phase names (`LoadRemainingFonts`
        // = 18) still fit; let the longest phase name set the width.
        let phase_w = self
            .phases
            .iter()
            .map(|t| t.phase.as_str().len())
            .max()
            .unwrap_or(5)
            .max("Phase".len());

        let show_notes = self.phases.iter().any(|t| t.notes.is_some());
        let notes_suffix_header = if show_notes { " │ Notes" } else { "" };
        let notes_suffix_sep = if show_notes { "─┼──────" } else { "" };

        let header = format!(
            "{:width$} │ Start ms │  Dur ms │ Critical{notes_suffix_header}",
            "Phase",
            width = phase_w,
        );
        let sep = format!(
            "{:─<width$}─┼──────────┼─────────┼─────────{notes_suffix_sep}",
            "",
            width = phase_w,
        );

        let mut out = String::new();
        let _ = writeln!(out, "{header}");
        let _ = writeln!(out, "{sep}");

        // Sort by started_at_ms so the table reads chronologically; matches
        // the spec example output and is friendlier to humans diffing runs.
        let mut rows: Vec<&PhaseTiming> = self.phases.iter().collect();
        rows.sort_by(|a, b| {
            a.started_at_ms
                .partial_cmp(&b.started_at_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for t in rows {
            let mark = if t.on_critical { "✓" } else { " " };
            let notes_cell = if show_notes {
                format!(" │ {}", t.notes.as_deref().unwrap_or(""))
            } else {
                String::new()
            };
            let _ = writeln!(
                out,
                "{name:width$} │ {start:>7.2}  │ {dur:>6.2}  │    {mark}   {notes_cell}",
                name = t.phase.as_str(),
                width = phase_w,
                start = t.started_at_ms,
                dur = t.duration_ms,
                mark = mark,
                notes_cell = notes_cell,
            );
        }
        let _ = writeln!(out, "{sep}");
        let _ = writeln!(
            out,
            "Critical path (serial sum): {:.2} ms",
            self.critical_path_ms()
        );
        let _ = writeln!(out, "First interactive:          {:.2} ms", self.first_interactive_ms);
        let _ = writeln!(out, "Wall clock (incl. async):   {:.2} ms", self.total_wall_clock_ms());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(phase: StartupPhase, start: f64, dur: f64) -> PhaseTiming {
        PhaseTiming {
            phase,
            started_at_ms: start,
            duration_ms: dur,
            on_critical: phase.is_critical(),
            notes: None,
        }
    }

    fn t_note(phase: StartupPhase, start: f64, dur: f64, note: &str) -> PhaseTiming {
        PhaseTiming {
            phase,
            started_at_ms: start,
            duration_ms: dur,
            on_critical: phase.is_critical(),
            notes: Some(note.into()),
        }
    }

    #[test]
    fn critical_path_sums_only_critical() {
        let r = StartupReport {
            phases: vec![
                t(StartupPhase::ReadFile, 0.0, 3.5),         // critical
                t(StartupPhase::InitGpuContext, 0.0, 80.0),  // critical
                t(StartupPhase::RenderSplash, 5.0, 4.0),     // NOT critical
                t(StartupPhase::DecodeImages, 50.0, 200.0),  // NOT critical
            ],
            first_interactive_ms: 100.0,
        };
        assert_eq!(r.critical_path_ms(), 3.5 + 80.0);
    }

    #[test]
    fn total_wall_clock_uses_latest_end() {
        let r = StartupReport {
            phases: vec![
                t(StartupPhase::ReadFile, 0.0, 3.0),
                t(StartupPhase::DecodeImages, 50.0, 200.0), // ends at 250 ms
                t(StartupPhase::ParseSchema, 3.0, 12.0),    // ends at 15 ms
            ],
            first_interactive_ms: 80.0,
        };
        assert_eq!(r.total_wall_clock_ms(), 250.0);
    }

    #[test]
    fn pretty_includes_every_phase_name_and_totals() {
        let r = StartupReport {
            phases: vec![
                t(StartupPhase::ReadFile, 0.0, 3.5),
                t(StartupPhase::ParseSchema, 3.5, 12.4),
                t(StartupPhase::DecodeImages, 50.0, 100.0),
            ],
            first_interactive_ms: 128.6,
        };
        let s = r.pretty();
        assert!(s.contains("ReadFile"));
        assert!(s.contains("ParseSchema"));
        assert!(s.contains("DecodeImages"));
        assert!(s.contains("Critical path"));
        assert!(s.contains("First interactive"));
        assert!(s.contains("Wall clock"));
        // Critical phases get the ✓ mark; non-critical do not.
        let read_line = s
            .lines()
            .find(|l| l.starts_with("ReadFile"))
            .expect("ReadFile row");
        assert!(read_line.contains("✓"));
        let decode_line = s
            .lines()
            .find(|l| l.starts_with("DecodeImages"))
            .expect("DecodeImages row");
        assert!(!decode_line.contains("✓"));
    }

    #[test]
    fn pretty_orders_rows_by_start_time() {
        let r = StartupReport {
            phases: vec![
                t(StartupPhase::ParseSchema, 3.5, 12.0),
                t(StartupPhase::ReadFile, 0.0, 3.5),
                t(StartupPhase::SeedStateGraph, 16.0, 1.0),
            ],
            first_interactive_ms: 17.0,
        };
        let s = r.pretty();
        let read_idx = s.find("ReadFile").unwrap();
        let parse_idx = s.find("ParseSchema").unwrap();
        let seed_idx = s.find("SeedStateGraph").unwrap();
        assert!(read_idx < parse_idx);
        assert!(parse_idx < seed_idx);
    }

    #[test]
    fn ended_at_ms_adds_start_and_duration() {
        let timing = t(StartupPhase::ParseSchema, 3.5, 12.4);
        assert!((timing.ended_at_ms() - 15.9).abs() < 1e-9);
    }

    #[test]
    fn empty_report_renders_without_panic() {
        let s = StartupReport::default().pretty();
        assert!(s.contains("Critical path"));
    }

    #[test]
    fn pretty_omits_notes_column_when_no_phase_has_notes() {
        let r = StartupReport {
            phases: vec![t(StartupPhase::ReadFile, 0.0, 1.0)],
            first_interactive_ms: 1.0,
        };
        let s = r.pretty();
        assert!(!s.contains("Notes"), "Notes column should be hidden:\n{s}");
    }

    #[test]
    fn pretty_renders_notes_column_when_any_phase_has_notes() {
        let r = StartupReport {
            phases: vec![
                t(StartupPhase::ReadFile, 0.0, 1.0),
                t_note(StartupPhase::InitGpuContext, 0.0, 80.0, "Metal"),
            ],
            first_interactive_ms: 80.0,
        };
        let s = r.pretty();
        assert!(s.contains("Notes"), "Notes header missing:\n{s}");
        assert!(s.contains("Metal"), "Notes content missing:\n{s}");
    }
}
