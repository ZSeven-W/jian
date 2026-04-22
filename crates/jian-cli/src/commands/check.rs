//! `jian check PATH [--json]`.
//!
//! Exit codes:
//! - `0` — parse succeeded with no warnings.
//! - `1` — parse succeeded but produced one or more warnings.
//! - `2` — parse / validation error (unsupported version, malformed
//!   JSON, …). Caller-visible `anyhow::Error`.

use crate::CheckArgs;
use anyhow::{anyhow, Context, Result};
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::error::LoadWarning;
use std::fs;
use std::process::ExitCode;

pub fn run(args: CheckArgs) -> Result<ExitCode> {
    let src =
        fs::read_to_string(&args.path).with_context(|| format!("read {}", args.path.display()))?;
    let loaded = jian_ops_schema::load_str(&src)
        .with_context(|| format!("parse {}", args.path.display()))?;

    // `load_str` only enforces the serde-derivable surface. Walk the
    // document and apply semantic checks that the .op spec requires
    // but which can't be expressed in serde annotations.
    if let Err(e) = semantic_check(&loaded.value) {
        if args.json {
            println!(
                "{}",
                serde_json::json!({
                    "severity": "error",
                    "kind": "semantic",
                    "detail": { "message": format!("{}", e) },
                })
            );
        } else {
            eprintln!("jian check: {} — semantic error: {}", args.path.display(), e);
        }
        // Same exit code as a parse failure (2) — the document is
        // structurally valid JSON but violates the .op contract.
        return Ok(ExitCode::from(2));
    }

    if args.json {
        print_json(&loaded.warnings);
    } else {
        print_human(&args.path.display().to_string(), &loaded.warnings);
    }

    Ok(if loaded.warnings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Post-deserialisation semantic checks. These express invariants the
/// `.op` spec requires but that `PenDocument`'s serde derives can't
/// enforce on their own (e.g. "top-level id is required when `app` is
/// set"). A real-app `.op` without `id` never ships, so we treat this
/// as a hard error at `check` time even though deserialisation itself
/// happily allows it.
fn semantic_check(doc: &PenDocument) -> Result<()> {
    if doc.app.is_some() && doc.id.as_deref().filter(|s| !s.is_empty()).is_none() {
        return Err(anyhow!(
            "document has `app` but no non-empty top-level `id`; one is required"
        ));
    }
    Ok(())
}

fn print_human(path: &str, warnings: &[LoadWarning]) {
    if warnings.is_empty() {
        println!("jian check: {} — OK, no diagnostics", path);
        return;
    }
    println!(
        "jian check: {} — {} diagnostic{}",
        path,
        warnings.len(),
        if warnings.len() == 1 { "" } else { "s" }
    );
    for (i, w) in warnings.iter().enumerate() {
        println!("  {}. {}", i + 1, render_warning(w));
    }
}

fn print_json(warnings: &[LoadWarning]) {
    for w in warnings {
        let (kind, detail) = warning_tuple(w);
        let line = serde_json::json!({
            "severity": "warning",
            "kind": kind,
            "detail": detail,
        });
        println!("{}", line);
    }
}

fn render_warning(w: &LoadWarning) -> String {
    match w {
        LoadWarning::UnknownField { path, field } => {
            format!("unknown field `{}` at {}", field, path)
        }
        LoadWarning::FutureFormatVersion {
            found,
            supported_max,
        } => {
            format!(
                "formatVersion {} is newer than supported ({}); behaviour may be undefined",
                found, supported_max
            )
        }
        LoadWarning::LogicModulesSkipped { reason } => {
            format!("logicModules skipped: {}", reason)
        }
        LoadWarning::InvalidExpression { path, expr, reason } => {
            format!("invalid expression at {}: `{}` — {}", path, expr, reason)
        }
    }
}

fn warning_tuple(w: &LoadWarning) -> (&'static str, serde_json::Value) {
    match w {
        LoadWarning::UnknownField { path, field } => (
            "unknown_field",
            serde_json::json!({ "path": path, "field": field }),
        ),
        LoadWarning::FutureFormatVersion {
            found,
            supported_max,
        } => (
            "future_format_version",
            serde_json::json!({ "found": found, "supported_max": supported_max }),
        ),
        LoadWarning::LogicModulesSkipped { reason } => (
            "logic_modules_skipped",
            serde_json::json!({ "reason": reason }),
        ),
        LoadWarning::InvalidExpression { path, expr, reason } => (
            "invalid_expression",
            serde_json::json!({ "path": path, "expr": expr, "reason": reason }),
        ),
    }
}
