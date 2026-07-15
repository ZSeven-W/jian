# Jian iOS Metal spike

This static library is the smallest C-callable consumer of Jian's borrowed
`CAMetalLayer` API. It creates one persistent Skia/Metal renderer and draws a
white frame with a centered red `DrawOp::Rect`.

## Build

Run from the Jian repository root:

```sh
cargo build -p jian-ios-spike --target aarch64-apple-ios --release
cargo build -p jian-ios-spike --target aarch64-apple-ios-sim --release
```

The artifacts are:

- Device: `target/aarch64-apple-ios/release/libjian_ios_spike.a`
- Apple-silicon simulator: `target/aarch64-apple-ios-sim/release/libjian_ios_spike.a`
- Header: `packaging/ios-spike/include/jian_ios_spike.h`

Use the archive matching the Xcode destination. The crate's `[lib]` uses
`crate-type = ["staticlib"]`; no dynamic Rust library is required.

This sandbox cannot resolve GitHub during a fresh release-profile build. The
matching prebuilt archives from the pinned checks are already cached, so the
exact offline commands used in this checkout are:

```sh
SKIA_BINARIES_URL='file:///Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios/debug/build/skia-bindings-0e75f33230027c8b/out/.cache/skia-binaries-{key}.tar.gz' cargo build -p jian-ios-spike --target aarch64-apple-ios --release
SKIA_BINARIES_URL='file:///Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios-sim/debug/build/skia-bindings-e30b2d66de29a275/out/.cache/skia-binaries-{key}.tar.gz' cargo build -p jian-ios-spike --target aarch64-apple-ios-sim --release
```

## Bare Xcode app settings

Set the app deployment target to iOS 14 or newer and use the standard `arm64`
architecture. Add the matching archive to **Link Binary With Libraries**, add
`packaging/ios-spike/include` to **Header Search Paths**, and expose
`jian_ios_spike.h` through the app's bridging header.

Add these values to **Other Linker Flags**:

```text
$(inherited) -lc++
-framework CoreFoundation -framework CoreGraphics -framework CoreText
-framework ImageIO -framework MobileCoreServices -framework UIKit
-framework Foundation -framework Metal -framework QuartzCore
```

`-ObjC`, `-all_load`, and `-force_load` are not required. Xcode supplies
`libSystem` normally.

## Minimal layer setup

The shell owns the layer. Configure it before calling Rust, keep it alive until
destroy returns, and do not replace it without destroying the renderer first:

```swift
import Metal
import QuartzCore
import UIKit

final class MetalSpikeView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    private var renderer: OpaquePointer?
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }

    override init(frame: CGRect) {
        super.init(frame: frame)
        metalLayer.device = MTLCreateSystemDefaultDevice()
        metalLayer.pixelFormat = .bgra8Unorm
        metalLayer.framebufferOnly = false
        renderer = jian_ios_spike_create(
            Unmanaged.passUnretained(metalLayer).toOpaque()
        )
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is unsupported") }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = window?.screen.scale ?? contentScaleFactor
        metalLayer.drawableSize = CGSize(
            width: bounds.width * scale,
            height: bounds.height * scale
        )
    }

    func drawRedFrame() {
        guard let renderer else { return }
        _ = jian_ios_spike_draw_red(renderer)
    }

    deinit { jian_ios_spike_destroy(renderer) }
}
```

Call `drawRedFrame()` only after the view is attached to a window and has a
non-zero `drawableSize`. A return value of `0` means no drawable was available;
retry on a later display-link tick. `-1` is an invalid-input/rendering error.
