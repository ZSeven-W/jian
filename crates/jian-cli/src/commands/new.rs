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

    let app_id = slugify(&args.name);
    let rendered = template_src
        .replace("{{APP_NAME}}", &args.name)
        .replace("{{APP_ID}}", &app_id);

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
}
