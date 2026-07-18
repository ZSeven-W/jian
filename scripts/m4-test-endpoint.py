"""Task 7 test endpoint: /ok.png completes immediately, /pending* accepts and
hangs (deterministic in-flight for the cancellation assertion), /error500 and
/stall exercise the HTTP error + timeout branches. Port 8477 (adb reverse)."""
import struct, time, zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def solid_png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))


OK_PNG = solid_png(64, 48, (0x1E, 0x66, 0xC8))  # blue, distinct from the local green


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        print("ENDPOINT:", self.path, flush=True)

    def do_GET(self):
        p = self.path.split("?")[0]
        if p == "/ok.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(OK_PNG)))
            self.send_header("X-Jian-Test", "ok")
            self.end_headers()
            self.wfile.write(OK_PNG)
        elif p == "/error500":
            body = b"deliberate server error"
            self.send_response(500)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif p in ("/pending", "/pending.png", "/stall"):
            # Accept then hang: stays in-flight deterministically.
            try:
                time.sleep(600)
            except Exception:
                pass
        else:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", 8477), Handler).serve_forever()
