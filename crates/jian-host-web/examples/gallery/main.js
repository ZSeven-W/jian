import { mountJian } from "/packages/jian-web/dist/index.js";

const canvas = document.querySelector("#jian");
const status = document.querySelector("#status");
let handle;
let fixture;

function report(message) {
  status.textContent = `${message}\n${status.textContent}`.slice(0, 2000);
}

function replaceHeroWithRelative(value) {
  if (Array.isArray(value)) return value.forEach(replaceHeroWithRelative);
  if (!value || typeof value !== "object") return;
  if (value.type === "image" && value.id === "hero") value.src = "hero.png";
  Object.values(value).forEach(replaceHeroWithRelative);
}

async function mount(withAssetBase) {
  handle?.dispose();
  handle = await mountJian(canvas, fixture, {
    wasmUrl: "/crates/jian-host-web/pkg/jian_host_web_bg.wasm",
    canvasKitUrl: "/packages/jian-web/canvaskit/canvaskit.js",
    assetBase: withAssetBase ? new URL("./", location.href).href : undefined,
    onWarning: (warning) => report(`warning: ${warning.message}`),
    onError: (error) => report(`error: ${error.message}`),
  });
  report(`mounted (${withAssetBase ? "assetBase present" : "assetBase absent"})`);
}

fixture = await fetch("./responsive-gallery.json").then((response) => response.json());
replaceHeroWithRelative(fixture);
await mount(true);

document.querySelector("#invalid").addEventListener("click", async () => {
  try {
    await handle.setDocument("{ invalid json");
  } catch (error) {
    report(`invalid reload rejected: ${error.message}`);
  }
});

document.querySelector("#without-assets").addEventListener("click", () => {
  mount(false).catch((error) => report(`remount failed: ${error.message}`));
});

document.querySelector("#context-loss").addEventListener("click", () => {
  const gl = canvas.getContext("webgl2") || canvas.getContext("webgl");
  const extension = gl?.getExtension("WEBGL_lose_context");
  if (!extension) return report("WEBGL_lose_context unavailable; use DevTools");
  extension.loseContext();
  setTimeout(() => extension.restoreContext(), 500);
  report("context loss requested; restore scheduled");
});

document.querySelector("#dispose").addEventListener("click", () => {
  handle?.dispose();
  handle = undefined;
  report("disposed");
});
