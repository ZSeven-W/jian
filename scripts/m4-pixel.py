"""Print `R,G,B` for one pixel of a PNG screencap: m4-pixel.py <png> <x> <y>.

Used by the M4 acceptance run to assert what actually reached the screen —
logcat can only show that bytes were accepted, not that they were painted.
std-only so the acceptance script needs no Python packages.
"""
import struct
import sys
import zlib


def pixel(path, x, y):
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    offset, idat, width, color = 8, b"", 0, 6
    while offset < len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        tag = data[offset + 4 : offset + 8]
        if tag == b"IHDR":
            width, height, _depth, color = struct.unpack(
                ">IIBB", data[offset + 8 : offset + 18]
            )
        elif tag == b"IDAT":
            idat += data[offset + 8 : offset + 8 + length]
        offset += 12 + length

    channels = 4 if color == 6 else 3
    raw = zlib.decompress(idat)
    stride = width * channels + 1
    if not 0 <= y < height or not 0 <= x < width:
        raise SystemExit(f"({x},{y}) outside {width}x{height}")

    # Scanlines are filtered relative to the previous one, so every row up to
    # `y` has to be reconstructed even though only one pixel is wanted.
    previous = bytearray(width * channels)
    for row in range(y + 1):
        filter_type = raw[row * stride]
        line = bytearray(raw[row * stride + 1 : (row + 1) * stride])
        for i in range(len(line)):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            up_left = previous[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + left) & 0xFF
            elif filter_type == 2:
                line[i] = (line[i] + up) & 0xFF
            elif filter_type == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xFF
            elif filter_type == 4:
                estimate = left + up - up_left
                da, db, dc = (
                    abs(estimate - left),
                    abs(estimate - up),
                    abs(estimate - up_left),
                )
                nearest = left if (da <= db and da <= dc) else (up if db <= dc else up_left)
                line[i] = (line[i] + nearest) & 0xFF
        previous = line
    base = x * channels
    return line[base], line[base + 1], line[base + 2]


if __name__ == "__main__":
    png, px, py = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    print("{},{},{}".format(*pixel(png, px, py)))
