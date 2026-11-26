"""Generates the Bento Desk icon: assets/bento.ico and assets/bento.png.

Every size is rendered natively from the vector description below (no
downscaling of a big bitmap), with analytic anti-aliasing. Sizes up to 32 px
snap the tile edges to the pixel grid so the small icons stay crisp.

Usage: python scripts/gen-icon.py   (needs Pillow and numpy)
"""

import io
import struct
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICO_SIZES = [16, 20, 24, 28, 30, 32, 36, 40, 48, 56, 60, 64, 72, 80, 96, 128, 256]
PNG_SIZE = 512

# Layout in a 256-unit design grid: base plate, then the four tiles
# (x0, y0, x1, y1) and their colours.
BASE = (14.0, 14.0, 242.0, 242.0)
BASE_RADIUS = 52.0
TILES = [
    ((30.0, 30.0, 145.0, 104.0), (0x00, 0x78, 0xD4)),  # blue
    ((156.0, 30.0, 226.0, 104.0), (0x10, 0xB9, 0x81)),  # green
    ((30.0, 115.0, 99.0, 226.0), (0xF5, 0x9E, 0x0B)),  # orange
    ((110.0, 115.0, 226.0, 226.0), (0xEC, 0x48, 0x99)),  # pink
]
TILE_RADIUS = 16.0
BASE_TOP = (0x30, 0x37, 0x45)
BASE_BOTTOM = (0x16, 0x1A, 0x22)

# Hand-tuned pixel layouts for the smallest sizes: base rect and radius,
# tile rects. Coordinates are pixel edges.
SMALL = {
    16: ((0, 0, 16, 16), 3.5, [(2, 2, 9, 7), (10, 2, 14, 7), (2, 8, 6, 14), (7, 8, 14, 14)], 1.2),
    20: ((0, 0, 20, 20), 4.5, [(3, 3, 12, 9), (13, 3, 17, 9), (3, 10, 8, 17), (9, 10, 17, 17)], 1.5),
    24: ((1, 1, 23, 23), 5.0, [(3, 3, 13, 10), (14, 3, 21, 10), (3, 11, 9, 21), (10, 11, 21, 21)], 1.8),
}


def grid(size, ss):
    """Sample coordinates (in output pixels) of an ss x ss supersampling grid."""
    o = (np.arange(size * ss) + 0.5) / ss
    return np.meshgrid(o, o)


def rrect_sdf(x, y, rect, r):
    x0, y0, x1, y1 = rect
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    hx, hy = (x1 - x0) / 2, (y1 - y0) / 2
    r = min(r, hx, hy)
    qx = np.abs(x - cx) - hx + r
    qy = np.abs(y - cy) - hy + r
    out = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0))
    inside = np.minimum(np.maximum(qx, qy), 0)
    return out + inside - r


def coverage(d, ss):
    """Analytic coverage from a distance in output pixels, then box-filtered."""
    c = np.clip(0.5 - d * 1.0, 0.0, 1.0) if ss == 1 else (d < 0).astype(np.float64)
    if ss > 1:
        h, w = c.shape
        c = c.reshape(h // ss, ss, w // ss, ss).mean(axis=(1, 3))
    return c


def down(a, ss):
    if ss == 1:
        return a
    h, w = a.shape[:2]
    return a.reshape(h // ss, ss, w // ss, ss, *a.shape[2:]).mean(axis=(1, 3))


def over(dst, rgb, alpha):
    """Composites a colour layer (rgb HxWx3 or 3-tuple, alpha HxW) over dst."""
    rgb = np.broadcast_to(np.asarray(rgb, dtype=np.float64), dst[..., :3].shape)
    a = alpha[..., None]
    da = dst[..., 3:4]
    out_a = a + da * (1 - a)
    out_rgb = np.where(out_a > 0, (rgb * a + dst[..., :3] * da * (1 - a)) / np.maximum(out_a, 1e-9), 0)
    dst[..., :3] = out_rgb
    dst[..., 3:4] = out_a


def vgrad(size, y0, y1, top, bottom):
    t = np.clip((np.arange(size) + 0.5 - y0) / max(y1 - y0, 1e-9), 0, 1)[:, None, None]
    top = np.asarray(top, dtype=np.float64) / 255
    bottom = np.asarray(bottom, dtype=np.float64) / 255
    return np.broadcast_to(top + (bottom - top) * t, (size, size, 3))


def shade(rgb, k):
    """k > 0 mixes towards white, k < 0 towards black."""
    c = np.asarray(rgb, dtype=np.float64)
    return tuple(c + (255 - c) * k) if k > 0 else tuple(c * (1 + k))


def blur(mask, sigma):
    """Separable Gaussian blur of a mask."""
    r = max(1, int(np.ceil(sigma * 3)))
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / max(sigma, 1e-6)) ** 2)
    k /= k.sum()
    out = np.apply_along_axis(lambda v: np.convolve(v, k, mode="same"), 0, mask)
    return np.apply_along_axis(lambda v: np.convolve(v, k, mode="same"), 1, out)


def shift(mask, dy):
    """Moves a mask down by dy pixels (fractional, linear interpolation)."""
    i = int(np.floor(dy))
    f = dy - i
    a = np.roll(mask, i, axis=0)
    b = np.roll(mask, i + 1, axis=0)
    out = a * (1 - f) + b * f
    out[: i + 1] = 0
    return out


def render(size):
    ss = 4
    x, y = grid(size, ss)
    img = np.zeros((size, size, 4))

    if size in SMALL:
        base, base_r, rects, tile_r = SMALL[size]
        tiles = [(rect, col) for rect, (_, col) in zip(rects, TILES)]
        rich = False
    else:
        s = size / 256
        snap = size <= 32
        fix = (lambda v: round(v * s)) if snap else (lambda v: v * s)
        base = tuple(fix(v) for v in BASE)
        base_r = BASE_RADIUS * s
        tiles = [(tuple(fix(v) for v in rect), col) for rect, col in TILES]
        tile_r = TILE_RADIUS * s
        rich = size >= 40

    s = size / 256
    base_d = rrect_sdf(x, y, base, base_r)
    base_m = coverage(base_d, ss)

    # Soft shadow under the plate.
    if rich:
        sh = blur(shift(base_m, 3.0 * s), 5.0 * s)
        over(img, (0, 0, 0), sh * 0.45)

    # Plate: vertical gradient plus a faint light from the top-left.
    plate = vgrad(size, base[1], base[3], BASE_TOP, BASE_BOTTOM).copy()
    if rich:
        cx, cy = down(x, ss), down(y, ss)
        glow = np.clip(1 - np.hypot(cx - base[0], cy - base[1]) / (size * 0.9), 0, 1) ** 2
        plate += glow[..., None] * 0.05
    over(img, plate, base_m)

    # Rim: light on the top edge, dark on the bottom edge (reads on light and
    # dark backgrounds alike).
    if size >= 28:
        w = max(1.0, 1.6 * s)
        ring = coverage(base_d, ss) - coverage(base_d + w, ss)
        ty = (down(y, ss) - base[1]) / (base[3] - base[1])
        over(img, (1, 1, 1), np.clip(ring * (0.22 * (1 - ty * 1.6)), 0, 1))
        over(img, (0, 0, 0), np.clip(ring * (0.35 * (ty * 1.6 - 0.6)), 0, 1))

    for rect, col in tiles:
        d = rrect_sdf(x, y, rect, tile_r)
        m = coverage(d, ss)
        if rich:
            sh = blur(shift(m, 2.5 * s), 3.5 * s) * base_m
            over(img, (0, 0, 0), sh * 0.40)
        top = shade(col, 0.16 if rich else 0.08)
        bottom = shade(col, -0.10 if rich else -0.04)
        over(img, vgrad(size, rect[1], rect[3], top, bottom), m)
        if rich:
            # Inner highlight along the top edge of the tile.
            w = max(1.0, 2.0 * s)
            ring = m - coverage(d + w, ss)
            ty = np.clip((down(y, ss) - rect[1]) / (rect[3] - rect[1]), 0, 1)
            over(img, (1, 1, 1), np.clip(ring * 0.40 * (1 - ty * 2.5), 0, 1))
            # Gentle sheen over the upper half.
            sheen = np.clip(1 - ty * 2.0, 0, 1) ** 2 * 0.10
            over(img, (1, 1, 1), m * sheen)

    rgba = np.clip(img * [255, 255, 255, 255] + 0.5, 0, 255).astype(np.uint8)
    return Image.fromarray(rgba, "RGBA")


def dib(im):
    """32-bit BMP frame for an ICO entry (BGRA bottom-up plus AND mask)."""
    w, h = im.size
    px = np.asarray(im)[::-1, :, [2, 1, 0, 3]]
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, w * h * 4, 0, 0, 0, 0)
    row = ((w + 31) // 32) * 4
    return header + px.tobytes() + bytes(row * h)


def write_ico(path, images):
    frames = []
    for im in images:
        if im.size[0] >= 64:
            buf = io.BytesIO()
            im.save(buf, "PNG", optimize=True)
            frames.append(buf.getvalue())
        else:
            frames.append(dib(im))
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for im, data in zip(images, frames):
        w = im.size[0]
        out += struct.pack("<BBBBHHII", w % 256, w % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    path.write_bytes(out + b"".join(frames))


def main():
    images = [render(s) for s in ICO_SIZES]
    write_ico(ROOT / "assets" / "bento.ico", images)
    render(PNG_SIZE).save(ROOT / "assets" / "bento.png", optimize=True)


if __name__ == "__main__":
    main()
