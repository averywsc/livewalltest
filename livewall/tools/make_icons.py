# Generates assets/app.ico and settings/icons/icon.{ico,png}: white play triangle on black.
import struct, zlib, os

def tri_cov(x, y, n):
    ax, ay, bx, by, cx, cy = 0.34, 0.24, 0.34, 0.76, 0.78, 0.50
    def side(px, py, x1, y1, x2, y2): return (px - x2) * (y1 - y2) - (x1 - x2) * (py - y2)
    hit = 0
    for sy in range(4):
        for sx in range(4):
            px = (x + (sx + 0.5) / 4) / n; py = (y + (sy + 0.5) / 4) / n
            d1 = side(px, py, ax, ay, bx, by); d2 = side(px, py, bx, by, cx, cy); d3 = side(px, py, cx, cy, ax, ay)
            neg = d1 < 0 or d2 < 0 or d3 < 0; pos = d1 > 0 or d2 > 0 or d3 > 0
            if not (neg and pos): hit += 1
    return hit / 16

def png(n):
    raw = bytearray()
    b = max(1, n // 32)
    for y in range(n):
        raw.append(0)
        for x in range(n):
            v = int(round(255 * tri_cov(x, y, n)))
            if x < b or y < b or x >= n - b or y >= n - b: v = 255
            raw += bytes((v, v, v, 255))
    def chunk(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', n, n, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b'')

sizes = [16, 32, 48, 64, 128, 256]
pngs = [png(s) for s in sizes]
head = struct.pack('<HHH', 0, 1, len(sizes))
off = 6 + 16 * len(sizes)
ents = b''
for s, p in zip(sizes, pngs):
    ents += struct.pack('<BBBBHHII', 0 if s == 256 else s, 0 if s == 256 else s, 0, 0, 1, 32, len(p), off)
    off += len(p)
ico = head + ents + b''.join(pngs)
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.makedirs(os.path.join(root, 'assets'), exist_ok=True)
os.makedirs(os.path.join(root, 'settings', 'icons'), exist_ok=True)
open(os.path.join(root, 'assets', 'app.ico'), 'wb').write(ico)
open(os.path.join(root, 'settings', 'icons', 'icon.ico'), 'wb').write(ico)
open(os.path.join(root, 'settings', 'icons', 'icon.png'), 'wb').write(pngs[-1])
print('icons written')
