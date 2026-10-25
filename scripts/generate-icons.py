#!/usr/bin/env python3
"""Generate raster app icons for MIX agent from its shared geometric mark."""

from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / "apps/web/public"
DESKTOP = ROOT / "apps/desktop/app/icons"
SCALE = 4
SIZE = 256 * SCALE


def create_icon(square_background: bool = False) -> Image.Image:
    image = Image.new("RGBA", (SIZE, SIZE), (31, 138, 112, 255))

    mask = Image.new("L", (SIZE, SIZE), 255 if square_background else 0)
    if not square_background:
        ImageDraw.Draw(mask).rounded_rectangle((0, 0, SIZE - 1, SIZE - 1), radius=58 * SCALE, fill=255)
    image.putalpha(mask)

    mark = Image.new("RGBA", (SIZE, SIZE))
    draw = ImageDraw.Draw(mark)

    def box(coords: tuple[float, float, float, float]) -> tuple[int, int, int, int]:
        return tuple(round(value * SCALE) for value in coords)

    def rounded(coords: tuple[float, float, float, float], radius: int, fill: tuple[int, ...], outline: tuple[int, ...] | None = None, width: int = 1) -> None:
        draw.rounded_rectangle(box(coords), radius=radius * SCALE, fill=fill, outline=outline, width=width * SCALE)

    # A minimal antenna, side pieces, and a friendly face remain clear at 32px.
    draw.line((128 * SCALE, 86 * SCALE, 128 * SCALE, 72 * SCALE), fill=(255, 255, 255, 255), width=8 * SCALE)
    draw.ellipse(box((117, 50, 139, 72)), fill=(255, 255, 255, 255))
    rounded((39, 125, 57, 166), 9, (255, 255, 255, 255))
    rounded((199, 125, 217, 166), 9, (255, 255, 255, 255))
    rounded((59, 86, 197, 203), 35, (255, 255, 255, 255))
    rounded((73, 104, 183, 187), 25, (12, 74, 61, 255))
    draw.ellipse(box((88.5, 132.5, 109.5, 153.5)), fill=(138, 230, 194, 255))
    draw.ellipse(box((146.5, 132.5, 167.5, 153.5)), fill=(138, 230, 194, 255))
    smile = []
    for step in range(33):
        t = step / 32
        inverse = 1 - t
        x = inverse**3 * 115 + 3 * inverse**2 * t * 120 + 3 * inverse * t**2 * 136 + t**3 * 141
        y = inverse**3 * 159 + 3 * inverse**2 * t * 174 + 3 * inverse * t**2 * 174 + t**3 * 159
        smile.append((round(x * SCALE), round(y * SCALE)))
    smile_width = round(5.5 * SCALE)
    draw.line(smile, fill=(138, 230, 194, 255), width=smile_width, joint="curve")
    for x, y in ((115, 159), (141, 159)):
        draw.ellipse(box((x - 2.75, y - 2.75, x + 2.75, y + 2.75)), fill=(138, 230, 194, 255))

    image = Image.alpha_composite(image, mark)
    return image.resize((1024, 1024), Image.Resampling.LANCZOS)


def save_png(icon: Image.Image, path: Path, size: int) -> None:
    icon.resize((size, size), Image.Resampling.LANCZOS).save(path, format="PNG", optimize=True)


def main() -> None:
    icon = create_icon()
    source_svg = WEB / "icon.svg"
    svg = source_svg.read_text(encoding="utf-8")
    for destination in (
        ROOT / "apps/MIX-agent/icon.svg",
        ROOT / "apps/desktop/app/icons/icon.svg",
        ROOT / "apps/desktop/ui/public/icon.svg",
    ):
        destination.write_text(svg, encoding="utf-8")
    for size in (192, 512):
        save_png(icon, WEB / f"icon-{size}.png", size)
    save_png(create_icon(square_background=True), WEB / "icon-maskable-512.png", 512)
    save_png(icon, WEB / "apple-touch-icon.png", 180)
    icon.resize((1024, 1024), Image.Resampling.LANCZOS).save(WEB / "favicon.ico", format="ICO", sizes=[(16, 16), (32, 32), (48, 48)])

    for name, size in {
        "icon-32.png": 32,
        "32x32.png": 32,
        "64x64.png": 64,
        "icon-128.png": 128,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon-256.png": 256,
        "icon-512.png": 512,
        "icon.png": 512,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }.items():
        save_png(icon, DESKTOP / name, size)
    icon.save(DESKTOP / "icon.ico", format="ICO", sizes=[(16, 16), (32, 32), (48, 48), (256, 256)])
    icon.save(DESKTOP / "icon.icns", format="ICNS")


if __name__ == "__main__":
    main()
