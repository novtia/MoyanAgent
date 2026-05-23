"""Generate app icons from public/luma_logo.png for Tauri bundle."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "public" / "luma_logo.png"
ICON_PNG = ROOT / "src-tauri" / "icons" / "icon.png"
FAVICON = ROOT / "public" / "favicon.png"
INSTALLER_DIR = ROOT / "src-tauri" / "icons" / "installer"
SIDEBAR_BMP = INSTALLER_DIR / "sidebar.bmp"
HEADER_BMP = INSTALLER_DIR / "header.bmp"
OUTPUT_SIZE = 1024
CORNER_RADIUS_RATIO = 0.22
SIDEBAR_SIZE = (164, 314)
HEADER_SIZE = (150, 57)
BG = (10, 10, 12)
ACCENT = (180, 230, 255)


def apply_round_corners(img: Image.Image, radius_ratio: float = CORNER_RADIUS_RATIO) -> Image.Image:
    """Clip image to a rounded rectangle with transparent outer corners."""
    size = img.size[0]
    radius = max(1, int(size * radius_ratio))
    mask = Image.new("L", img.size, 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)
    rounded = img.copy()
    rounded.putalpha(Image.composite(img.split()[3], Image.new("L", img.size, 0), mask))
    return rounded


def make_square_source() -> None:
    img = Image.open(SRC).convert("RGBA")
    w, h = img.size
    size = max(w, h)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 255))
    canvas.paste(img, ((size - w) // 2, (size - h) // 2), img)
    canvas = canvas.resize((OUTPUT_SIZE, OUTPUT_SIZE), Image.Resampling.LANCZOS)
    canvas = apply_round_corners(canvas)
    ICON_PNG.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(ICON_PNG)
    print(f"wrote {ICON_PNG} ({canvas.size[0]}x{canvas.size[1]})")


def run_tauri_icon() -> None:
    subprocess.run(
        "npx tauri icon "
        f'"{ICON_PNG}" '
        f'-o "{ICON_PNG.parent}"',
        cwd=ROOT,
        check=True,
        shell=True,
    )


def copy_favicon() -> None:
    src = ICON_PNG.parent / "32x32.png"
    if src.exists():
        FAVICON.write_bytes(src.read_bytes())
        print(f"wrote {FAVICON}")


def make_installer_assets() -> None:
    """Generate NSIS sidebar/header BMP images for the custom installer wizard."""
    logo = Image.open(SRC).convert("RGBA")
    INSTALLER_DIR.mkdir(parents=True, exist_ok=True)

    sidebar = Image.new("RGB", SIDEBAR_SIZE, BG)
    draw = ImageDraw.Draw(sidebar)
    draw.rectangle((0, 0, SIDEBAR_SIZE[0] - 1, 4), fill=ACCENT)
    logo_w = 96
    logo_h = max(1, int(logo_w * logo.height / logo.width))
    logo_resized = logo.resize((logo_w, logo_h), Image.Resampling.LANCZOS)
    x = (SIDEBAR_SIZE[0] - logo_w) // 2
    y = (SIDEBAR_SIZE[1] - logo_h) // 2 - 20
    sidebar.paste(logo_resized, (x, y), logo_resized)
    sidebar.save(SIDEBAR_BMP)
    print(f"wrote {SIDEBAR_BMP}")

    header = Image.new("RGB", HEADER_SIZE, BG)
    header_draw = ImageDraw.Draw(header)
    header_draw.rectangle((0, HEADER_SIZE[1] - 3, HEADER_SIZE[0] - 1, HEADER_SIZE[1] - 1), fill=ACCENT)
    header_logo_w = 36
    header_logo_h = max(1, int(header_logo_w * logo.height / logo.width))
    header_logo = logo.resize((header_logo_w, header_logo_h), Image.Resampling.LANCZOS)
    header.paste(header_logo, (8, (HEADER_SIZE[1] - header_logo_h) // 2), header_logo)
    header.save(HEADER_BMP)
    print(f"wrote {HEADER_BMP}")


def main() -> int:
    if not SRC.exists():
        print(f"missing source logo: {SRC}", file=sys.stderr)
        return 1
    make_square_source()
    run_tauri_icon()
    copy_favicon()
    make_installer_assets()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
