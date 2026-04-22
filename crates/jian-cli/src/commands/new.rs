//! `jian new NAME [--template counter|form] [--path DIR]` — scaffold
//! a fresh Jian project by writing an embedded template to disk with
//! `{{APP_NAME}}` / `{{APP_ID}}` placeholders substituted.

use crate::NewArgs;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const TPL_COUNTER: &str = include_str!("../templates/counter.op");
const TPL_FORM: &str = include_str!("../templates/form.op");

pub fn run(args: NewArgs) -> Result<ExitCode> {
    let template_src = match args.template.as_str() {
        "counter" => TPL_COUNTER,
        "form" => TPL_FORM,
        other => {
            return Err(anyhow!(
                "unknown template `{}` (available: counter, form)",
                other
            ));
        }
    };

    // `name` is used as both the project-directory name (when `--path`
    // is omitted) and the app id. Reject anything that would escape the
    // current working directory or embed path separators.
    validate_name(&args.name)?;

    let app_id = slugify(&args.name);
    if app_id.is_empty() {
        return Err(anyhow!(
            "cannot derive a slug from `{}` — name must contain ASCII alphanumerics",
            args.name
        ));
    }

    let rendered = render_template(template_src, &args.name, &app_id);

    let dir = args.path.unwrap_or_else(|| PathBuf::from(&args.name));
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    let op_path = dir.join("app.op");
    if op_path.exists() {
        return Err(anyhow!(
            "{} already exists; refusing to overwrite",
            op_path.display()
        ));
    }
    fs::write(&op_path, &rendered).with_context(|| format!("write {}", op_path.display()))?;

    println!(
        "jian new: scaffolded {} at {} from template `{}`",
        args.name,
        dir.display(),
        args.template,
    );
    Ok(ExitCode::SUCCESS)
}

/// Guard `--path`-less `jian new NAME` against filesystem traversal.
/// `NAME` doubles as a directory name, so anything that isn't a single
/// plain segment is rejected.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("project name cannot be empty"));
    }
    if name.contains(['/', '\\']) {
        return Err(anyhow!(
            "project name `{}` must not contain path separators",
            name
        ));
    }
    if name == "." || name == ".." || name.starts_with("..") {
        return Err(anyhow!(
            "project name `{}` must not be `.` or start with `..`",
            name
        ));
    }
    Ok(())
}

/// Single-pass placeholder substitution. Scans the template once so a
/// user-supplied name containing `{{APP_ID}}` can't accidentally be
/// re-interpreted as a second-round substitution.
fn render_template(template: &str, app_name: &str, app_id: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(idx) = rest.find("{{") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + 2..];
        if let Some(end) = after.find("}}") {
            let key = &after[..end];
            let replacement = match key {
                "APP_NAME" => Some(app_name),
                "APP_ID" => Some(app_id),
                _ => None,
            };
            if let Some(value) = replacement {
                out.push_str(value);
                rest = &after[end + 2..];
            } else {
                // Unknown placeholder — preserve literally so we don't
                // silently swallow a typo.
                out.push_str(&rest[idx..idx + 2 + end + 2]);
                rest = &after[end + 2..];
            }
        } else {
            // Unterminated `{{` — emit the rest verbatim and stop.
            out.push_str(&rest[idx..]);
            break;
        }
    }
    out.push_str(rest);
    out
}

/// Very small kebab-case slug: lowercase, ASCII-alphanumeric + `-`,
/// collapse runs, strip leading/trailing dashes. Good enough for
/// template app ids; real projects should supply their own via
/// `jian.toml` later (Plan 10+).
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            out.push(lc);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  My.App  "), "my-app");
        assert_eq!(slugify("CamelCase_thing"), "camelcase-thing");
    }

    #[test]
    fn validate_name_rejects_traversal() {
        assert!(validate_name("ok").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("../foo").is_err());
        assert!(validate_name("foo/bar").is_err());
        assert!(validate_name("foo\\bar").is_err());
    }

    #[test]
    fn render_template_substitutes_both_placeholders() {
        let out = render_template("{{APP_NAME}} ({{APP_ID}})", "My App", "my-app");
        assert_eq!(out, "My App (my-app)");
    }

    #[test]
    fn render_template_does_not_re_expand_user_content() {
        // User name literally contains `{{APP_ID}}` — must NOT be
        // re-interpreted as a placeholder on a second pass.
        let out = render_template("name={{APP_NAME}} id={{APP_ID}}", "{{APP_ID}}", "slug");
        assert_eq!(out, "name={{APP_ID}} id=slug");
    }

    #[test]
    fn render_template_preserves_unknown_placeholders() {
        let out = render_template("a={{OTHER}} b={{APP_ID}}", "n", "s");
        assert_eq!(out, "a={{OTHER}} b=s");
    }
}
