# jian-host-web

CanvasKit-backed `wasm32-unknown-unknown` host for Jian. This crate deliberately
has no `#[wasm_bindgen(start)]`; the npm loader initializes CanvasKit and calls
the exported mount API explicitly.

## Build

```sh
cargo check -p jian-host-web --target wasm32-unknown-unknown
wasm-pack build crates/jian-host-web --target web
```

## Browser tests

Chrome or Chromium must be installed and discoverable by `wasm-pack`.

```sh
wasm-pack test --headless --chrome crates/jian-host-web
```

## Browser limitations

- CSS transforms on the mounted canvas are unsupported. Resize the canvas with
  ordinary CSS width/height rules; pointer mapping assumes an untransformed CSS
  box.
- Browser activation can expire across asynchronous work. Put
  activation-sensitive actions such as `open_url` and clipboard writes before
  actions that await timers, network requests, or other promises.
