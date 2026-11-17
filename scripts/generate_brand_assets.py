#!/usr/bin/env python3

import argparse
import math
import random
from pathlib import Path

from PIL import Image, ImageDraw

GREEN = (166, 209, 137, 190)
BLUE = (139, 185, 232, 180)
MUTED = (157, 159, 150, 135)


def hand_line(draw, points, color, width, rng):
    for first, second in zip(points, points[1:]):
        x1, y1 = first
        x2, y2 = second
        distance = math.hypot(x2 - x1, y2 - y1)
        steps = max(2, int(distance / 4))
        drift = rng.uniform(-0.45, 0.45)
        previous = None
        for index in range(steps + 1):
            ratio = index / steps
            envelope = math.sin(math.pi * ratio)
            point = (
                x1 + (x2 - x1) * ratio + rng.uniform(-0.22, 0.22) + drift * envelope,
                y1 + (y2 - y1) * ratio + rng.uniform(-0.22, 0.22) - drift * envelope,
            )
            if previous is not None:
                pressure = 0.55 + 0.45 * math.sin(math.pi * ratio)
                draw.line([previous, point], fill=color, width=max(1, round(width * pressure)))
            previous = point


def arc_points(box, start, end):
    left, top, right, bottom = box
    cx, cy = (left + right) / 2, (top + bottom) / 2
    rx, ry = (right - left) / 2, (bottom - top) / 2
    steps = max(12, int(abs(end - start) / 3))
    return [
        (
            cx + rx * math.cos(math.radians(start + (end - start) * index / steps)),
            cy + ry * math.sin(math.radians(start + (end - start) * index / steps)),
        )
        for index in range(steps + 1)
    ]


def four_point_mark(draw, x, y, radius, color, width, rng):
    hand_line(draw, [(x, y - radius), (x + 1, y - 2), (x, y + radius)], color, width, rng)
    hand_line(draw, [(x - radius, y), (x - 2, y + 1), (x + radius, y)], color, width, rng)


def add_matte_grain(image, count, seed):
    rng = random.Random(seed)
    overlay = Image.new("RGBA", image.size)
    draw = ImageDraw.Draw(overlay)
    for index in range(count):
        x = rng.randrange(image.width)
        y = rng.randrange(image.height)
        if index % 2:
            color = (235, 240, 225, rng.randrange(2, 8))
        else:
            color = (0, 0, 0, rng.randrange(3, 10))
        draw.point((x, y), fill=color)
    return Image.alpha_composite(image.convert("RGBA"), overlay)


def decorate_banner(base):
    scale = 3
    rng = random.Random(1206)
    overlay = Image.new("RGBA", (base.width * scale, base.height * scale))
    draw = ImageDraw.Draw(overlay)

    def stroke(points, color=GREEN, width=2):
        hand_line(draw, [(x * scale, y * scale) for x, y in points], color, width * scale, rng)

    stroke(arc_points((46, 48, 332, 300), 192, 286), GREEN, 2)
    stroke(arc_points((46, 48, 332, 300), 18, 102), GREEN, 2)
    stroke([(414, 283), (520, 286), (632, 284), (744, 287), (866, 283)], BLUE, 2)
    stroke([(416, 290), (524, 292), (634, 290)], (139, 185, 232, 85), 1)
    stroke(arc_points((952, 42, 1164, 304), 205, 320), MUTED, 2)
    stroke([(1040, 319), (1070, 323), (1100, 318)], MUTED, 2)
    four_point_mark(draw, 1090 * scale, 72 * scale, 12 * scale, GREEN, 2 * scale, rng)
    four_point_mark(draw, 365 * scale, 64 * scale, 7 * scale, BLUE, 1.5 * scale, rng)

    overlay = overlay.resize(base.size, Image.Resampling.LANCZOS)
    return add_matte_grain(Image.alpha_composite(base.convert("RGBA"), overlay), 2400, 1207)


def decorate_icon(base):
    scale = 2
    rng = random.Random(1212)
    overlay = Image.new("RGBA", (base.width * scale, base.height * scale))
    draw = ImageDraw.Draw(overlay)
    hand_line(
        draw,
        [(x * scale, y * scale) for x, y in arc_points((730, 126, 884, 280), 218, 302)],
        (166, 209, 137, 120),
        3 * scale,
        rng,
    )
    overlay = overlay.resize(base.size, Image.Resampling.LANCZOS)
    return add_matte_grain(Image.alpha_composite(base.convert("RGBA"), overlay), 5200, 1213)


def save_png(image, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, format="PNG", optimize=True, compress_level=9)


def main():
    parser = argparse.ArgumentParser(description="Decorate rendered LazyKeys HTML/CSS logo assets with Pillow.")
    parser.add_argument("--banner-base", type=Path, required=True)
    parser.add_argument("--icon-base", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent

    with Image.open(args.banner_base) as source:
        banner = decorate_banner(source)
    with Image.open(args.icon_base) as source:
        icon = decorate_icon(source)

    save_png(banner, root / "docs/assets/lazykeys-readme.png")
    save_png(icon, root / "docs/assets/lazykeys-icon.png")
    save_png(icon.resize((64, 64), Image.Resampling.LANCZOS), root / "public/favicon.png")


if __name__ == "__main__":
    main()
