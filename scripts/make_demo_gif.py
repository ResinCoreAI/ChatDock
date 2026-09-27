"""Turns the frames recorded by src/demo.js into the README animation and stills.

    python scripts/make_avatars.py <frames dir>          (pictures for the two senders)
    electron . --demo-frames=<frames dir> --profile=<fresh scratch dir> --no-occlusion
    python scripts/make_demo_gif.py <frames dir> scripts/demo-scene.png <out dir>

Each frame = scene + ChatDock's windows at their screen positions + a drawn cursor (with a ripple
on clicks). Writes <out>/demo.gif and a few full-size PNG stills.
"""
import json
import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

GIF_WIDTH = 1200
FRAME_MS = 50  # 20 fps
CURSOR_SCALE = 1.5
ARROW = [(0, 0), (0, 21), (5, 16.2), (8.6, 24.4), (12, 23), (8.5, 15), (15, 15)]


def cursor_sprite(scale):
    pts = [(x * scale + 4, y * scale + 4) for x, y in ARROW]
    w, h = int(20 * scale + 10), int(28 * scale + 10)
    shadow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon([(x + 1.5, y + 2) for x, y in pts], fill=(0, 0, 0, 110))
    shadow = shadow.filter(ImageFilter.GaussianBlur(2))
    arrow = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(arrow)
    d.polygon(pts, fill=(255, 255, 255, 255))
    d.line(pts + [pts[0]], fill=(17, 17, 17, 255), width=max(1, round(scale)), joint="curve")
    shadow.alpha_composite(arrow)
    return shadow  # the arrow tip sits at (4, 4)


def caption_font(size):
    for name in ("segoeuisb.ttf", "seguisb.ttf", "segoeuib.ttf", "arialbd.ttf"):
        p = os.path.join(r"C:\Windows\Fonts", name)
        if os.path.exists(p):
            return ImageFont.truetype(p, size)
    return ImageFont.load_default()


def draw_caption(canvas, text, alpha):
    """Dark pill with white text, lower middle: clear of the pop-ups, the dock and the game HUD."""
    font = caption_font(round(canvas.width * 0.019))
    probe = ImageDraw.Draw(canvas)
    l, t, r, b = probe.textbbox((0, 0), text, font=font)
    pad_x, pad_y = round(font.size * 0.9), round(font.size * 0.5)
    w, h = r - l + pad_x * 2, b - t + pad_y * 2
    x, y = (canvas.width - w) // 2, round(canvas.height * 0.79)
    layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    d.rounded_rectangle((x, y, x + w, y + h), radius=h // 2, fill=(12, 12, 20, int(205 * alpha)),
                        outline=(255, 255, 255, int(40 * alpha)), width=2)
    d.text((x + pad_x - l, y + pad_y - t), text, font=font, fill=(255, 255, 255, int(255 * alpha)))
    canvas.alpha_composite(layer)


def shadow_strip(height, width=30, left_side=True, strength=120):
    """Soft shadow next to the panel's inner edge (the real window has none of its own)."""
    strip = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    px = strip.load()
    for x in range(width):
        t = (x + 1) / width if left_side else 1 - x / width
        a = int(strength * t ** 2.2)
        for y in range(height):
            px[x, y] = (0, 0, 0, a)
    return strip


def main():
    frames_dir, scene_path, out_dir = sys.argv[1:4]
    os.makedirs(out_dir, exist_ok=True)
    meta = json.load(open(os.path.join(frames_dir, "frames.json"), encoding="utf-8"))
    wa = meta["workArea"]
    frames = meta["frames"]
    clicks = meta["clicks"]
    captions = meta.get("captions", [])
    scene = Image.open(scene_path).convert("RGBA").resize((wa["width"], wa["height"]))
    arrow = cursor_sprite(CURSOR_SCALE)
    cache = {}

    def layer(name):
        if name not in cache:
            cache[name] = Image.open(os.path.join(frames_dir, name)).convert("RGBA")
        return cache[name]

    def compose(fr, with_caption=True):
        canvas = scene.copy()
        for L in fr["layers"]:
            img = layer(L["file"])
            if img.size != (L["w"], L["h"]):
                img = img.resize((L["w"], L["h"]), Image.LANCZOS)
            x, y = L["x"] - wa["x"], L["y"] - wa["y"]
            if L["name"] == "panel":
                docked_right = L["x"] + L["w"] >= wa["x"] + wa["width"] - 1
                strip = shadow_strip(L["h"], left_side=docked_right)
                canvas.alpha_composite(strip, (x - strip.width, y) if docked_right else (x + L["w"], y))
            if L.get("opacity", 1) < 1:
                img = img.copy()
                img.putalpha(img.getchannel("A").point(lambda a: int(a * L["opacity"])))
            # layers may hang off the screen edge while sliding: paste the visible part only
            left, top = max(0, -x), max(0, -y)
            right, bottom = min(img.width, canvas.width - x), min(img.height, canvas.height - y)
            if right > left and bottom > top:
                canvas.alpha_composite(img.crop((left, top, right, bottom)), (x + left, y + top))
        cx, cy = fr["cursor"]["x"] - wa["x"], fr["cursor"]["y"] - wa["y"]
        for c in clicks:
            p = (fr["t"] - c) / 420
            if 0 <= p <= 1:
                ring = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
                r = 10 + 30 * p
                ImageDraw.Draw(ring).ellipse((cx - r, cy - r, cx + r, cy + r),
                                             outline=(120, 170, 255, int(230 * (1 - p))), width=5)
                canvas.alpha_composite(ring)
        canvas.alpha_composite(arrow, (min(max(cx - 4, 0), canvas.width - 1), min(max(cy - 4, 0), canvas.height - 1)))
        current = [c for c in captions if c["t"] <= fr["t"]] if with_caption else []
        if current:
            c = current[-1]
            first = c is captions[0]
            draw_caption(canvas, c["text"], 1 if first else min(1, (fr["t"] - c["t"]) / 220))
        return canvas

    # resample to a steady frame rate (nearest recorded frame at each tick)
    start, end = frames[0]["t"], frames[-1]["t"]
    picks, i = [], 0
    for t in range(start, end + 1, FRAME_MS):
        while i + 1 < len(frames) and abs(frames[i + 1]["t"] - t) <= abs(frames[i]["t"] - t):
            i += 1
        picks.append(frames[i])

    size = (GIF_WIDTH, round(wa["height"] * GIF_WIDTH / wa["width"]))
    out_frames, durations = [], []
    prev = None
    for fr in picks:
        small = compose(fr).convert("RGB").resize(size, Image.LANCZOS)
        if prev is not None and small.tobytes() == prev.tobytes():
            durations[-1] += FRAME_MS
            continue
        prev = small
        out_frames.append(small)
        durations.append(FRAME_MS)
    durations[-1] += 1500  # rest on the last frame before looping

    palette_src = Image.new("RGB", (size[0], size[1] * 4))
    for k, idx in enumerate([len(out_frames) // 5, len(out_frames) // 3, len(out_frames) // 2, 4 * len(out_frames) // 5]):
        palette_src.paste(out_frames[idx], (0, size[1] * k))
    palette = palette_src.quantize(colors=255, method=Image.Quantize.MEDIANCUT)
    gif_frames = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in out_frames]
    gif = os.path.join(out_dir, "demo.gif")
    gif_frames[0].save(gif, save_all=True, append_images=gif_frames[1:], duration=durations, loop=0, optimize=True)
    print("gif", gif, len(gif_frames), "frames", round(os.path.getsize(gif) / 1e6, 2), "MB", size)

    # full-size stills at chosen moments (seconds from the start)
    for name, sec in [("popups", 3.4), ("chat-open", 5.9), ("settings", 8.4), ("left-dock", 11.4)]:
        t = start + sec * 1000
        fr = min(frames, key=lambda f: abs(f["t"] - t))
        compose(fr, with_caption=False).convert("RGB").save(os.path.join(out_dir, f"still-{name}.png"))
    print("stills written")


if __name__ == "__main__":
    main()
