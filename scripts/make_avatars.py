"""Two made-up sender pictures for the README demo (see src/demo.js).

    python scripts/make_avatars.py <frames dir>
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

FONTS = r"C:\Windows\Fonts"


def font(size):
    for name in ("LeelaUIb.ttf", "LeelawUI.ttf", "segoeuib.ttf"):
        p = os.path.join(FONTS, name)
        if os.path.exists(p):
            return ImageFont.truetype(p, size)
    return ImageFont.load_default()


def avatar(path, letter, top, bottom):
    s = 192
    grad = Image.new("RGBA", (s, s))
    d = ImageDraw.Draw(grad)
    for y in range(s):
        t = y / (s - 1)
        d.line([(0, y), (s, y)], fill=tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,))
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).ellipse((0, 0, s - 1, s - 1), fill=255)
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    im.paste(grad, (0, 0), mask)
    f = font(96)
    d = ImageDraw.Draw(im)
    left, top_, right, bottom_ = d.textbbox((0, 0), letter, font=f)
    d.text(((s - (right - left)) / 2 - left, (s - (bottom_ - top_)) / 2 - top_), letter, font=f, fill="white")
    im.resize((96, 96), Image.LANCZOS).save(path)


if __name__ == "__main__":
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    avatar(os.path.join(out, "avatar-1.png"), "M", (255, 154, 139), (255, 106, 136))
    avatar(os.path.join(out, "avatar-2.png"), "ส", (106, 178, 255), (88, 101, 242))
    print("avatars written to", out)
