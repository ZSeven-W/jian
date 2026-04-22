//! `jian unpack INPUT OUTPUT_DIR` — extract every entry from a
//! `.op.pack` zip into the output directory. Inverse of `pack`.
//!
//! The zip-slip guard validates each entry name by its structural
//! `std::path::Component`s rather than substring matching: that rules
//! out Windows drive / UNC prefixes (`C:\`, `\\server\share\`),
//! POSIX absolute paths (`/etc/passwd`), and actual parent-traversals
//! (`../escape`) — while letting benign names like `foo/..bar/baz` or
//! `..intro.md` (dotfile-ish) through.

use crate::UnpackArgs;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

pub fn run(args: UnpackArgs) -> Result<ExitCode> {
    let file =
        fs::File::open(&args.input).with_context(|| format!("open {}", args.input.display()))?;
    let mut zr = zip::ZipArchive::new(file)?;

    fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("mkdir {}", args.output_dir.display()))?;

    let mut count = 0usize;
    for i in 0..zr.len() {
        let mut entry = zr.by_index(i)?;
        let name = entry.name().to_owned();
        let safe_rel = safe_entry_path(&name)
            .with_context(|| format!("unsafe entry name in archive: {}", name))?;
        let out_path = args.output_dir.join(&safe_rel);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&out_path)
                .with_context(|| format!("write {}", out_path.display()))?;
            io::copy(&mut entry, &mut out)?;
        }
        count += 1;
    }

    println!(
        "jian unpack: extracted {} entr{} to {}",
        count,
        if count == 1 { "y" } else { "ies" },
        args.output_dir.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Return `Ok(relative_path)` if every component of `name` is a plain
/// file/directory segment; otherwise return an error naming the
/// offending piece. Rejects:
/// - absolute paths (POSIX `/` root or Windows drive / UNC prefix)
/// - `..` parent-traversals
/// - embedded NUL bytes
fn safe_entry_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        return Err(anyhow!("empty entry name"));
    }
    if name.contains('\0') {
        return Err(anyhow!("entry name contains NUL byte"));
    }
    let p = Path::new(name);
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {} // `./foo` flattens to `foo`
            Component::ParentDir => {
                return Err(anyhow!("parent-directory component in `{}`", name));
            }
            Component::RootDir => {
                return Err(anyhow!("absolute path in `{}`", name));
            }
            Component::Prefix(_) => {
                return Err(anyhow!("drive / UNC prefix in `{}`", name));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(anyhow!("entry `{}` has no normal components", name));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_segments_pass() {
        assert_eq!(
            safe_entry_path("manifest.json").unwrap().to_str().unwrap(),
            "manifest.json"
        );
        let nested = safe_entry_path("assets/images/a.png").unwrap();
        // Component-wise compare so the test works on both `/` and `\`
        // path separators.
        let components: Vec<_> = nested
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(components, vec!["assets", "images", "a.png"]);
    }

    #[test]
    fn benign_double_dot_substrings_pass() {
        // `..bar` is a literal filename, not a traversal. The old
        // substring check rejected this; the component check accepts.
        assert!(safe_entry_path("foo/..bar/baz.txt").is_ok());
        assert!(safe_entry_path("..intro.md").is_ok());
    }

    #[test]
    fn parent_traversal_is_rejected() {
        assert!(safe_entry_path("../etc/passwd").is_err());
        assert!(safe_entry_path("a/../../b").is_err());
    }

    #[test]
    fn posix_absolute_is_rejected() {
        assert!(safe_entry_path("/etc/passwd").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_and_unc_prefix_are_rejected() {
        assert!(safe_entry_path("C:\\Windows\\system32\\evil.dll").is_err());
        assert!(safe_entry_path("\\\\server\\share\\foo.txt").is_err());
    }

    #[test]
    fn nul_byte_is_rejected() {
        assert!(safe_entry_path("foo\0bar").is_err());
    }

    #[test]
    fn empty_or_all_curdir_is_rejected() {
        assert!(safe_entry_path("").is_err());
        assert!(safe_entry_path("./.").is_err());
    }
}
