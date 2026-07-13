export default function init(
  input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module,
): Promise<unknown>;

export interface WasmJianHandle {
  setDocument(document: string | object): Promise<void>;
  dispose(): void;
}

export function mount_jian(
  canvas: HTMLCanvasElement,
  document: string | object,
  options: object,
): Promise<WasmJianHandle>;
