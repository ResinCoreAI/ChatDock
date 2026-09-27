"""Generate ChatDock icons (app .ico/.png and tray icons) with Pillow.

Run:  python scripts/make_icons.py
"""
import math
import os

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets")
S = 1024  # supersampled canvas; everything is downscaled with LANCZOS for anti-aliasing

IG_TO_MSG = [
    (0.00, (254, 218, 117)),
    (0.28, (250, 126, 30)),
    (0.52, (214, 41, 118)),
    (0.76, (150, 47, 191)),
    (1.00, (8, 102, 255)),
]


def gradient_in_box(stops, angle_deg, box):
    """Full-canvas image whose gradient spans only `box` (x0, y0, x1, y1), so a shape gets the whole range."""
    x0, y0, x1, y1 = [int(round(v)) for v in box]
    g = gradient(stops, angle_deg, size=max(x1 - x0, y1 - y0))
    canvas = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    canvas.paste(g, (x0, y0))
    return canvas


def gradient(stops, angle_deg, size=S):
    """Linear gradient computed at low resolution, then upscaled (it is smooth anyway)."""
    n = 256
    img = Image.new("RGB", (n, n))
    px = img.load()
    a = math.radians(angle_deg)
    dx, dy = math.cos(a), math.sin(a)
    projs = [x * dx + y * dy for x, y in ((0, 0), (n, 0), (0, n), (n, n))]
    lo, hi = min(projs), max(projs)
    for y in range(n):
        for x in range(n):
            t = ((x * dx + y * dy) - lo) / (hi - lo)
            for i in range(len(stops) - 1):
                t0, c0 = stops[i]
                t1, c1 = stops[i + 1]
                if t <= t1 or i == len(stops) - 2:
                    k = 0 if t1 == t0 else max(0.0, min(1.0, (t - t0) / (t1 - t0)))
                    px[x, y] = tuple(int(round(c0[j] + (c1[j] - c0[j]) * k)) for j in range(3))
                    break
    return img.resize((size, size), Image.BICUBIC).convert("RGBA")


def layer(fill, mask):
    if isinstance(fill, tuple):
        lay = Image.new("RGBA", (S, S), fill)
    else:
        lay = fill.copy()
    lay.putalpha(mask)
    return lay


def mask_of(draw_fn):
    m = Image.new("L", (S, S), 0)
    draw_fn(ImageDraw.Draw(m))
    return m


def make_master(unread=False):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Dark rounded-square base
    base = mask_of(lambda d: d.rounded_rectangle([8, 8, S - 9, S - 9], radius=int(S * 0.23), fill=255))
    img = Image.alpha_composite(img, layer(gradient([(0, (44, 46, 62)), (1, (13, 14, 19))], 65), base))

    # Chat bubble with Instagram -> Messenger gradient
    cx, cy, r = S * 0.405, S * 0.475, S * 0.285

    def bubble(d):
        d.ellipse([cx - r, cy - r * 0.9, cx + r, cy + r * 0.9], fill=255)
        d.polygon(
            [(cx - r * 0.70, cy + r * 0.45), (cx - r * 0.98, cy + r * 1.16), (cx - r * 0.12, cy + r * 0.84)],
            fill=255,
        )

    bubble_box = (cx - r, cy - r * 0.9, cx + r, cy + r * 1.16)
    img = Image.alpha_composite(img, layer(gradient_in_box(IG_TO_MSG, 50, bubble_box), mask_of(bubble)))

    # Three dots inside the bubble
    def dots(d):
        rr = S * 0.036
        for i in (-1, 0, 1):
            x = cx + i * S * 0.105
            d.ellipse([x - rr, cy - rr, x + rr, cy + rr], fill=255)

    img = Image.alpha_composite(img, layer((255, 255, 255, 255), mask_of(dots)))

    # The white edge tab (the thing you click on the right side of the screen)
    def pill(d):
        x0, x1 = S * 0.755, S * 0.865
        d.rounded_rectangle([x0, S * 0.25, x1, S * 0.75], radius=(x1 - x0) / 2, fill=255)

    img = Image.alpha_composite(img, layer((255, 255, 255, 255), mask_of(pill)))

    if unread:
        # Badge sits on the bubble's upper-right shoulder, clear of the white tab
        bx, by = S * 0.585, S * 0.235

        def ring(d):
            R = S * 0.165
            d.ellipse([bx - R, by - R, bx + R, by + R], fill=255)

        def dot(d):
            R = S * 0.12
            d.ellipse([bx - R, by - R, bx + R, by + R], fill=255)

        # Dark ring "cuts" the badge out of the pink bubble so it stays readable at 16 px
        img = Image.alpha_composite(img, layer((24, 25, 34, 255), mask_of(ring)))
        img = Image.alpha_composite(img, layer((255, 48, 80, 255), mask_of(dot)))

    return img


def main():
    os.makedirs(OUT, exist_ok=True)
    master = make_master()
    icon256 = master.resize((256, 256), Image.LANCZOS)
    icon256.save(os.path.join(OUT, "icon.png"))
    icon256.save(
        os.path.join(OUT, "icon.ico"),
        format="ICO",
        sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    tray_sizes = [(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48)]
    icon256.save(os.path.join(OUT, "tray.ico"), format="ICO", sizes=tray_sizes)
    unread256 = make_master(unread=True).resize((256, 256), Image.LANCZOS)
    unread256.save(os.path.join(OUT, "tray-unread.ico"), format="ICO", sizes=tray_sizes)
    unread256.save(os.path.join(OUT, "icon-unread.png"))
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
