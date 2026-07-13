# M2 responsive gallery

From the repository root:

```sh
wasm-pack build crates/jian-host-web --target web --out-dir ../../packages/jian-web/dist --out-name jian_host_web
cd packages/jian-web && npm install && npm run build
cd ../.. && python3 -m http.server 8080
```

Open `http://localhost:8080/crates/jian-host-web/examples/gallery/`.

Manual smoke: resize the window below 480px, between 481–1024px, and above
1024px; confirm both constraint movement and page-variant swaps. Click the
rendered text input and commit CJK text. Trigger `WEBGL_lose_context` with the
button or DevTools and confirm the dirty frame repaints after restore. Exercise
invalid `setDocument`, remount without `assetBase`, then dispose and confirm no
further frames/fetches/listeners remain.
