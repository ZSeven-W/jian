# Jian iOS Player

This directory is a source-only SwiftUI/UIKit shell for `jian-engine-ffi`. XcodeGen creates the project; the Rust archive remains an external build input and is not copied into this directory.

The app loads `Resources/m1_acceptance.op`, which is a byte-for-byte copy of `crates/jian-core/tests/fixtures/m1_acceptance.json`. `JianPlayerView` owns its `CAMetalLayer`; the pointer passed to Jian is borrowed and is valid until `jian_suspend`/`jian_destroy` returns. UIKit, CADisplayLink, all Jian calls, and all callback reactions run on the main thread. Callback payloads are copied synchronously, then reactions are dispatched asynchronously so callbacks never re-enter the ABI.

## Build inputs

From the repository root, build the archive matching the destination if it is not already present:

```bash
cd /Users/kayshen/Workspace/ZSeven-W/jian
cargo build -p jian-engine-ffi --release --target aarch64-apple-ios-sim --features metal,textlayout
cargo build -p jian-engine-ffi --release --target aarch64-apple-ios --features metal,textlayout
```

The resulting archives are:

- Simulator: `/Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios-sim/release/libjian_engine_ffi.a`
- Device: `/Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios/release/libjian_engine_ffi.a`

Generate the project (do this again after changing `project.yml`):

```bash
cd /Users/kayshen/Workspace/ZSeven-W/jian/packaging/ios-player
xcodegen generate --spec project.yml
```

## Simulator build and run

On this host, pass the SDK, destination, library search path, and linker flags explicitly. XcodeGen's framework dependencies do not supply the Rust archive search path. Replace `<sim-id>` with an installed iOS 26.4 simulator UUID:

```bash
cd /Users/kayshen/Workspace/ZSeven-W/jian/packaging/ios-player
xcodebuild \
  -project JianPlayer.xcodeproj \
  -scheme JianPlayer \
  -configuration Release \
  -sdk iphonesimulator26.4 \
  -destination 'platform=iOS Simulator,id=<sim-id>' \
  -derivedDataPath "$PWD/.derived-data" \
  LIBRARY_SEARCH_PATHS=/Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios-sim/release \
  HEADER_SEARCH_PATHS=/Users/kayshen/Workspace/ZSeven-W/jian/crates/jian-engine-ffi/include \
  OTHER_LDFLAGS='$(inherited) -ljian_engine_ffi -lc++ -framework CoreFoundation -framework CoreGraphics -framework CoreText -framework ImageIO -framework MobileCoreServices -framework UIKit -framework Foundation -framework Metal -framework QuartzCore' \
  build

xcrun simctl install <sim-id> "$PWD/.derived-data/Build/Products/Release-iphonesimulator/JianPlayer.app"
xcrun simctl launch <sim-id> dev.jian.player
```

## Real-device build

Use the device archive and replace `<device-id>` with the attached phone's destination identifier. Signing values may be supplied by the orchestrator or selected in Xcode:

```bash
cd /Users/kayshen/Workspace/ZSeven-W/jian/packaging/ios-player
xcodebuild \
  -project JianPlayer.xcodeproj \
  -scheme JianPlayer \
  -configuration Release \
  -sdk iphoneos26.4 \
  -destination 'platform=iOS,id=<device-id>' \
  -derivedDataPath "$PWD/.derived-data-device" \
  LIBRARY_SEARCH_PATHS=/Users/kayshen/Workspace/ZSeven-W/jian/target/aarch64-apple-ios/release \
  HEADER_SEARCH_PATHS=/Users/kayshen/Workspace/ZSeven-W/jian/crates/jian-engine-ffi/include \
  OTHER_LDFLAGS='$(inherited) -ljian_engine_ffi -lc++ -framework CoreFoundation -framework CoreGraphics -framework CoreText -framework ImageIO -framework MobileCoreServices -framework UIKit -framework Foundation -framework Metal -framework QuartzCore' \
  build
```

## Coordinate and lifecycle contract

The engine viewport is `view.bounds.size` in logical UIKit points. `CAMetalLayer.drawableSize` is `bounds × contentsScale` in physical pixels, but touch locations are passed directly from `UITouch.location(in:)` without multiplying by scale. Therefore pointer input and all returned text geometry share surface-logical points with a top-left origin.

`layoutSubviews` configures the Metal layer, creates the engine once, attaches the borrowed layer, and calls `jian_resize` for later bounds/scale changes (including rotation). Safe-area and keyboard occlusion are separate logical-point channels. Backgrounding suspends the borrowed surface; foregrounding resumes it. Teardown synchronously suspends and destroys the engine.

CADisplayLink is paused before every frame. A redraw callback caused by a mutation arms the next display tick; a callback from `jian_frame` either schedules the absolute monotonic `next_wake_ms` or leaves the link idle. Touch timestamps and frame timestamps both use `CACurrentMediaTime() × 1000`.

## Source-only validation

This does not generate a project or link an app. It checks the YAML/resource contract, compiles the bridging header, and type-checks every Swift source against the iOS simulator SDK and the checked-in `jian.h`:

```bash
cd /Users/kayshen/Workspace/ZSeven-W/jian
bash packaging/ios-player/Tests/validate_sources.sh
```
