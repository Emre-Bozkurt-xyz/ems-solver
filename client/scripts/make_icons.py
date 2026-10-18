"""Render the app icon source and the tray state icons. Needs Pillow.

    python client/scripts/make_icons.py
    npx tauri icon src-tauri/icons/app-icon.png   (from client/, regenerates bundle icons)
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ICONS = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

TOP, BOTTOM = (99, 102, 241), (124, 58, 237)
BADGES = {
    "idle": None,
    "buffered": (14, 165, 233),
    "busy": (245, 158, 11),
    "error": (239, 68, 68),
}


def font(size: int) -> ImageFont.FreeTypeFont:
    for name in ("segoeuib.ttf", "arialbd.ttf", "DejaVuSans-Bold.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default(size)


def base(size: int, grey: bool = False) -> Image.Image:
    # Draw at 4x and downsample for smooth edges.
    s = size * 4
    gradient = Image.new("RGB", (s, s))
    px = gradient.load()
    for y in range(s):
        t = y / (s - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
        if grey:
            g = round(sum(color) / 3 * 0.75)
            color = (g, g, g)
        for x in range(s):
            px[x, y] = color

    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, s - 1, s - 1), radius=s // 4.5, fill=255)
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    img.paste(gradient, (0, 0), mask)

    draw = ImageDraw.Draw(img)
    f = font(round(s * 0.66))
    draw.text((s / 2, s / 2 + s * 0.02), "E", font=f, fill="white", anchor="mm")
    return img.resize((size, size), Image.LANCZOS)


def with_badge(img: Image.Image, color) -> Image.Image:
    s = img.width * 4
    big = img.resize((s, s), Image.LANCZOS)
    draw = ImageDraw.Draw(big)
    r = s * 0.22
    cx, cy = s - r - s * 0.02, s - r - s * 0.02
    draw.ellipse((cx - r - s * 0.05, cy - r - s * 0.05, cx + r + s * 0.05, cy + r + s * 0.05), fill=(255, 255, 255, 255))
    draw.ellipse((cx - r, cy - r, cx + r, cy + r), fill=color + (255,))
    return big.resize((img.width, img.height), Image.LANCZOS)


def main() -> None:
    (ICONS / "tray").mkdir(parents=True, exist_ok=True)
    base(1024).save(ICONS / "app-icon.png")
    for name, badge in BADGES.items():
        icon = base(64)
        if badge:
            icon = with_badge(icon, badge)
        icon.save(ICONS / "tray" / f"{name}.png")
    base(64, grey=True).save(ICONS / "tray" / "setup.png")
    print("wrote", ICONS)


if __name__ == "__main__":
    main()
