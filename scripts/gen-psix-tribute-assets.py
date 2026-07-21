#!/usr/bin/env python3
"""Generate the PNG assets for the psix-tribute skin.

Deterministic procedural art (no external sources): run from the repo root
to (re)create skins/psix-tribute/assets/*.png. Requires Pillow.

All images use power-of-two dimensions (PSP texture requirement).
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "skins" / "psix-tribute" / "assets"

ORANGE = (245, 130, 15)
ORANGE_HI = (255, 170, 40)
LIME = (140, 235, 50)
CHROME = (24, 24, 26)
CHROME_HI = (46, 46, 50)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def save(img, name):
    OUT.mkdir(parents=True, exist_ok=True)
    img.save(OUT / name, optimize=True)
    print(f"wrote {OUT / name} ({img.width}x{img.height})")


def watermark():
    """128x128 orbital ring emblem, white-on-transparent (tinted via alpha)."""
    s = 4  # supersample
    size = 128 * s
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = size // 2
    # Outer ring.
    d.ellipse(
        [c - 56 * s, c - 56 * s, c + 56 * s, c + 56 * s],
        outline=(255, 255, 255, 255),
        width=6 * s,
    )
    # Inner filled orb, offset for the PSIX "eclipse" look.
    d.ellipse(
        [c - 26 * s, c - 34 * s, c + 26 * s, c + 18 * s],
        fill=(255, 255, 255, 200),
    )
    # Orbit swoosh: wide elliptical arc crossing the ring.
    d.arc(
        [c - 62 * s, c - 20 * s, c + 62 * s, c + 44 * s],
        start=200,
        end=340,
        fill=(255, 255, 255, 230),
        width=5 * s,
    )
    save(img.resize((128, 128), Image.LANCZOS), "watermark.png")


def bar(name, flip):
    """512x32 shaped chrome bar: angled notch on the right + accent line."""
    w, h = 512, 32
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    full_h = h
    thin_h = 20
    notch_x = 384  # silhouette steps down past here
    notch_w = 40
    # Bar silhouette: full height, then a diagonal step to a thinner strip.
    poly = [
        (0, 0),
        (w, 0),
        (w, thin_h),
        (notch_x + notch_w, thin_h),
        (notch_x, full_h),
        (0, full_h),
    ]
    d.polygon(poly, fill=(255, 255, 255, 255))
    mask = img.split()[3]
    # Vertical chrome gradient inside the silhouette.
    grad = Image.new("RGBA", (w, h))
    gd = ImageDraw.Draw(grad)
    for y in range(h):
        gd.line([(0, y), (w, y)], fill=lerp(CHROME_HI, CHROME, y / (h - 1)) + (235,))
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    out.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(out)
    # Orange accent along the shaped bottom edge.
    d.line([(0, full_h - 2), (notch_x, full_h - 2)], fill=ORANGE + (255,), width=2)
    d.line(
        [(notch_x, full_h - 2), (notch_x + notch_w, thin_h - 2)],
        fill=ORANGE + (255,),
        width=2,
    )
    d.line([(notch_x + notch_w, thin_h - 2), (w, thin_h - 2)], fill=ORANGE + (255,), width=2)
    if flip:
        out = out.transpose(Image.FLIP_TOP_BOTTOM)
    save(out, name)


def tab(name, active):
    """64x16 angled tab pill (parallelogram, PSIX-style)."""
    s = 4
    w, h = 64 * s, 16 * s
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    slant = 5 * s
    poly = [(slant, s), (w - s, s), (w - slant - s, h - s), (s, h - s)]
    if active:
        # Orange gradient fill via row clipping.
        grad = Image.new("RGBA", (w, h))
        gd = ImageDraw.Draw(grad)
        for y in range(h):
            gd.line([(0, y), (w, y)], fill=lerp(ORANGE_HI, ORANGE, y / (h - 1)) + (255,))
        mask = Image.new("L", (w, h), 0)
        ImageDraw.Draw(mask).polygon(poly, fill=255)
        img.paste(grad, (0, 0), mask)
        d.polygon(poly, outline=(255, 220, 170, 255), width=s)
    else:
        d.polygon(poly, fill=CHROME_HI + (190,))
        d.polygon(poly, outline=(90, 90, 96, 255), width=s)
    save(img.resize((64, 16), Image.LANCZOS), name)


def titlebar():
    """64x16 nine-patch titlebar: rounded top corners, orange baseline."""
    s = 4
    w, h = 64 * s, 16 * s
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, w - 1, h - 1], radius=5 * s, fill=(255, 255, 255, 255))
    d.rectangle([0, h // 2, w - 1, h - 1], fill=(255, 255, 255, 255))  # square bottom
    mask = img.split()[3]
    grad = Image.new("RGBA", (w, h))
    gd = ImageDraw.Draw(grad)
    for y in range(h):
        gd.line([(0, y), (w, y)], fill=lerp((58, 58, 64), (30, 30, 34), y / (h - 1)) + (255,))
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    out.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(out)
    d.line([(0, h - s), (w, h - s)], fill=ORANGE + (255,), width=s)
    save(out.resize((64, 16), Image.LANCZOS), "titlebar.png")


def frame():
    """32x32 nine-patch window frame: chrome border, dark center."""
    w = h = 32
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w - 1, h - 1], fill=(52, 52, 58, 255))
    d.rectangle([0, 0, w - 1, h - 1], outline=ORANGE + (180,), width=1)
    d.rectangle([3, 3, w - 4, h - 4], fill=(20, 20, 22, 255))
    save(img, "frame.png")


def cursor():
    """16x16 arrow cursor, white fill with orange outline; hotspot (1, 1)."""
    s = 8
    w = h = 16 * s
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    arrow = [
        (1 * s, 1 * s),
        (1 * s, 12 * s),
        (4 * s, 9 * s),
        (6 * s, 14 * s),
        (8 * s, 13 * s),
        (6 * s, 8 * s),
        (10 * s, 8 * s),
    ]
    d.polygon(arrow, fill=(245, 245, 240, 255), outline=ORANGE + (255,), width=s)
    save(img.resize((16, 16), Image.LANCZOS), "cursor.png")


if __name__ == "__main__":
    watermark()
    bar("bar_top.png", flip=False)
    bar("bar_bottom.png", flip=True)
    tab("tab_active.png", active=True)
    tab("tab_inactive.png", active=False)
    titlebar()
    frame()
    cursor()
