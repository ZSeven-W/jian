//! `jian unpack INPUT OUTPUT_DIR` — extract every entry from a
//! `.op.pack` zip into the output directory. Inverse of `pack`.

use crate::UnpackArgs;
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Read, Write};
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
        // Guard against zip-slip: reject absolute or parent-traversing
        // entry names.
        if name.starts_with('/') || name.contains("..") {
            anyhow::bail!("unsafe entry name in archive: {}", name);
        }
        let out_path = args.output_dir.join(&name);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&out_path)
                .with_context(|| format!("write {}", out_path.display()))?;
            io::copy(&mut entry, &mut out)?;
            let _ = out.flush();
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

#[allow(dead_code)]
fn _unused_read<R: Read>(_: R) {}
