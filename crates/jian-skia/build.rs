//! Build script for `jian-skia`.
//!
//! Sole job today: link Windows' `advapi32` when the `textlayout`
//! feature is on. skia-bindings 0.78's `skunicode_icu.lib` calls
//! `RegOpenKeyExW` / `RegQueryValueExW` / friends from
//! `uprv_detectWindowsTimeZone` but does not declare `advapi32` as a
//! transitive linker input, so an MSVC link of any binary that
//! pulls in skparagraph fails with `LNK2019` for those five
//! symbols. Track upstream <https://github.com/rust-skia/rust-skia>;
//! delete this script once skia-bindings advertises the dep itself.
//!
//! Uses `CARGO_CFG_TARGET_OS` + `CARGO_FEATURE_TEXTLAYOUT` because
//! `cfg!()` inside a build script reads the *host* cfg, not the
//! target's. Cross-compiles to Windows from a non-Windows host
//! still need this link line.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let textlayout = std::env::var("CARGO_FEATURE_TEXTLAYOUT").is_ok();
    if target_os == "windows" && textlayout {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
