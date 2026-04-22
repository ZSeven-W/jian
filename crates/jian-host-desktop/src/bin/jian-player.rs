//! `jian-player PATH` — load a `.op` file, build the runtime, and print
//! the scene's bounding box as a smoke test.
//!
//! The full winit event loop lands under the `run` feature. This MVP
//! binary validates that every piece of the stack (schema → document
//! → layout → capability gate) wires together end-to-end and is what
//! Plan 8 T11 asks for in the interim.

use std::env;
use std::fs;
use std::process::ExitCode;

use jian_core::Runtime;
use jian_host_desktop::DesktopHost;
use jian_ops_schema::load_str;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: jian-player <path/to/app.op>");
            return ExitCode::from(2);
        }
    };

    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("jian-player: failed to read {}: {}", path, e);
            return ExitCode::from(1);
        }
    };

    let schema = match load_str(&src) {
        Ok(doc) => doc.value,
        Err(e) => {
            eprintln!("jian-player: schema parse error: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut rt = match Runtime::new_from_document(schema) {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("jian-player: runtime build error: {}", e);
            return ExitCode::from(1);
        }
    };
    if let Err(e) = rt.build_layout((800.0, 600.0)) {
        eprintln!("jian-player: layout error: {}", e);
        return ExitCode::from(1);
    }
    rt.rebuild_spatial();

    let host = DesktopHost::new(rt, path.as_str());
    println!(
        "jian-player: loaded {} ({} nodes); initial size {}x{}",
        host.title(),
        host.runtime
            .document
            .as_ref()
            .map(|d| d.tree.nodes.len())
            .unwrap_or(0),
        host.initial_size().width,
        host.initial_size().height,
    );
    ExitCode::SUCCESS
}
