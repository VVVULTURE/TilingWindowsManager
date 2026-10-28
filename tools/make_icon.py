"""Generate assets/twm.ico: four rounded tiles in the default theme's
cyan-to-green gradient (rgba(33ccff) -> rgba(00ff99)). Pure Python, no
image libraries; sizes 16-256, the 256 px image stored as PNG.

    python tools/make_icon.py
"""
import pathlib
import struct
import zlib

A, B = (0x33, 0xCC, 0xFF), (0x00, 0xFF, 0x99)


def pixel(x, y, n):
    """Premultiplied-free RGBA of pixel centre (x, y) for an n x n icon."""
    gap = max(1.0, n / 16)
    tile = (n - 3 * gap) / 2
    radius = tile * 0.22
    px, py = x + 0.5, y + 0.5
    cov = 0.0
    for tx in (gap, 2 * gap + tile):
        for ty in (gap, 2 * gap + tile):
            # Signed distance to a rounded rectangle.
            cx, cy = tx + tile / 2, ty + tile / 2
            qx = abs(px - cx) - (tile / 2 - radius)
            qy = abs(py - cy) - (tile / 2 - radius)
            outside = (max(qx, 0) ** 2 + max(qy, 0) ** 2) ** 0.5
            d = outside + min(max(qx, qy), 0) - radius
            cov = max(cov, min(1.0, max(0.0, 0.5 - d)))
    t = (px + py) / (2 * n)
    r, g, b = (round(a * (1 - t) + c * t) for a, c in zip(A, B))
    return r, g, b, round(255 * cov)


def png(n):
    rows = b"".join(b"\0" + bytes(c for x in range(n) for c in pixel(x, y, n)) for y in range(n))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")


def dib(n):
    # BITMAPINFOHEADER with doubled height (XOR + AND masks), bottom-up BGRA.
    head = struct.pack("<IiiHHIIiiII", 40, n, 2 * n, 1, 32, 0, 0, 0, 0, 0, 0)
    xor = b"".join(bytes((b, g, r, a)) for y in reversed(range(n)) for (r, g, b, a) in (pixel(x, y, n) for x in range(n)))
    stride = ((n + 31) // 32) * 4
    return head + xor + b"\0" * stride * n


def main():
    sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    images = [png(n) if n >= 256 else dib(n) for n in sizes]
    out = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    for n, data in zip(sizes, images):
        out += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out += b"".join(images)
    dest = pathlib.Path(__file__).resolve().parents[1] / "assets" / "twm.ico"
    dest.parent.mkdir(exist_ok=True)
    dest.write_bytes(out)
    print(f"wrote {dest} ({len(out)} bytes)")


if __name__ == "__main__":
    main()
