import initWasm, { mount_jian, type WasmJianHandle } from "./jian_host_web.js";

export interface JianWarning {
  kind: string;
  message: string;
  source?: string;
}

export interface JianError {
  kind: string;
  message: string;
  source?: string;
}

export interface JianFont {
  family: string;
  data: ArrayBuffer;
}

export interface JianMountOptions {
  canvasKitUrl?: string;
  wasmUrl?: string | URL;
  fonts?: JianFont[];
  assetBase?: string;
  onWarning?: (warning: JianWarning) => void;
  onError?: (error: JianError) => void;
}

export interface JianHandle {
  setDocument(document: string | object): Promise<void>;
  dispose(): void;
}

let wasmReady: Promise<unknown> | undefined;
let canvasKitReady: Promise<void> | undefined;

function injectCanvasKit(scriptUrl: string): Promise<void> {
  if ((globalThis as typeof globalThis & { CanvasKitInit?: unknown }).CanvasKitInit) {
    return Promise.resolve();
  }
  canvasKitReady ??= new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = scriptUrl;
    script.async = true;
    script.addEventListener("load", () => resolve(), { once: true });
    script.addEventListener(
      "error",
      () => reject(new Error(`failed to load CanvasKit from ${scriptUrl}`)),
      { once: true },
    );
    document.head.appendChild(script);
  });
  return canvasKitReady;
}

class Handle implements JianHandle {
  #inner: WasmJianHandle | undefined;

  constructor(inner: WasmJianHandle) {
    this.#inner = inner;
  }

  setDocument(document: string | object): Promise<void> {
    if (!this.#inner) return Promise.reject(new Error("Jian handle is disposed"));
    return this.#inner.setDocument(document);
  }

  dispose(): void {
    this.#inner?.dispose();
    this.#inner = undefined;
  }
}

export async function mountJian(
  canvas: HTMLCanvasElement,
  documentValue: string | object,
  options: JianMountOptions = {},
): Promise<JianHandle> {
  if (!(canvas instanceof HTMLCanvasElement)) {
    throw new TypeError("mountJian requires an HTMLCanvasElement");
  }
  const canvasKitUrl =
    options.canvasKitUrl ?? new URL("../canvaskit/canvaskit.js", import.meta.url).href;
  const wasmUrl =
    options.wasmUrl ?? new URL("./jian_host_web_bg.wasm", import.meta.url);
  await injectCanvasKit(canvasKitUrl);
  wasmReady ??= initWasm(wasmUrl);
  await wasmReady;
  const inner = await mount_jian(canvas, documentValue, {
    ...options,
    canvasKitUrl,
    wasmUrl: undefined,
  });
  return new Handle(inner);
}
