#[cfg(all(feature = "desktop", not(target_os = "android")))]
fn main() -> Result<(), winit::error::EventLoopError> {
    jian_gallery::desktop::run()
}

#[cfg(not(all(feature = "desktop", not(target_os = "android"))))]
fn main() {
    println!("jian-gallery built without the desktop runner for this target");
}
