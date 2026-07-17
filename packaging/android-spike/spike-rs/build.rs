fn main() {
    // khronos-egl's `static` linking needs the Android system libraries; with
    // `no-pkg-config` we emit the links ourselves.
    println!("cargo:rustc-link-lib=EGL");
    println!("cargo:rustc-link-lib=GLESv2");
    println!("cargo:rustc-link-lib=android");
}
