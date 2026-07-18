"""Generate the checked-in Beatblock Online icon assets.

The mark follows the project sketch: an internet globe with Beatblock's round
character icon tucked into the lower-right as a sublabel. The game version uses
hard pixel edges; the installer version adds color and anti-aliasing.
"""

from __future__ import annotations

import argparse
import io
from pathlib import Path

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
ONLINE_PNG = ROOT / "mod" / "shared" / "assets" / "online.png"
INSTALLER_PNG = ROOT / "companion" / "assets" / "installer.png"
INSTALLER_ICO = ROOT / "companion" / "assets" / "installer.ico"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)


def draw_online_icon() -> Image.Image:
    """Draw a 72 px globe with Beatblock's native black/white keyline style."""

    image = Image.new("RGBA", (72, 72), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    black = (0, 0, 0, 255)
    white = (255, 255, 255, 255)

    def keyed_ellipse(bounds: tuple[int, int, int, int], outer: int = 7, inner: int = 3) -> None:
        draw.ellipse(bounds, outline=black, width=outer)
        draw.ellipse(bounds, outline=white, width=inner)

    def keyed_arc(
        bounds: tuple[int, int, int, int],
        start: int,
        end: int,
        outer: int = 7,
        inner: int = 3,
    ) -> None:
        draw.arc(bounds, start, end, fill=black, width=outer)
        draw.arc(bounds, start, end, fill=white, width=inner)

    def keyed_line(points: tuple[tuple[int, int], ...], outer: int = 7, inner: int = 3) -> None:
        draw.line(points, fill=black, width=outer, joint="curve")
        draw.line(points, fill=white, width=inner, joint="curve")

    # Native menu art combines white forms with a chunky black edge. Drawing the
    # keyline first keeps the icon visible over both pale and dark menu frames.
    globe = (3, 3, 57, 57)
    keyed_ellipse(globe)
    keyed_ellipse((17, 3, 43, 57), outer=6, inner=2)
    keyed_arc((4, 12, 56, 37), 8, 172, outer=6, inner=2)
    keyed_line(((5, 31), (55, 31)), outer=6, inner=2)

    # Clear the overlap before adding a compact version of Beatblock's actual
    # game icon: round block, two eyes, connector, and the protective halo.
    draw.ellipse((34, 34, 72, 72), fill=(0, 0, 0, 0))
    draw.ellipse((40, 47, 68, 72), fill=black)
    draw.ellipse((44, 50, 64, 70), fill=white)
    draw.rectangle((50, 41, 58, 51), fill=black)
    draw.rectangle((52, 43, 56, 50), fill=white)
    keyed_arc((35, 34, 72, 66), 202, 342, outer=9, inner=4)
    draw.rectangle((49, 56, 52, 65), fill=black)
    draw.rectangle((57, 56, 60, 65), fill=black)
    return image


def validate_online_icon(image: Image.Image) -> None:
    """Guard the contrast contract that prevents another invisible menu icon."""

    if image.size != (72, 72) or image.mode != "RGBA":
        raise ValueError("Online icon must remain a 72x72 RGBA image")
    pixels = list(image.get_flattened_data())
    black_pixels = sum(pixel == (0, 0, 0, 255) for pixel in pixels)
    white_pixels = sum(pixel == (255, 255, 255, 255) for pixel in pixels)
    transparent_pixels = sum(pixel[3] == 0 for pixel in pixels)
    if black_pixels < 350 or white_pixels < 250:
        raise ValueError("Online icon must retain substantial black keylines and white forms")
    if transparent_pixels < 1_500:
        raise ValueError("Online icon needs transparent breathing room around the silhouette")


def draw_installer_icon(size: int = 1024) -> Image.Image:
    """Draw the colored application mark at a high-resolution working size."""

    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    unit = size / 256

    def p(value: float) -> int:
        return round(value * unit)

    # The deep circular tile stays distinct against both light and dark Windows
    # themes, while transparent corners keep the icon from feeling like a box.
    draw.ellipse(
        (p(12), p(12), p(244), p(244)),
        fill=(15, 21, 36, 255),
        outline=(46, 59, 83, 255),
        width=max(1, p(5)),
    )
    draw.ellipse(
        (p(23), p(23), p(233), p(233)),
        outline=(31, 42, 62, 255),
        width=max(1, p(2)),
    )

    cyan = (76, 225, 245, 255)
    white = (244, 248, 252, 255)
    navy = (15, 21, 36, 255)

    # Globe grid mirrors the in-game silhouette, with enough spacing to survive
    # Windows' 16 px downsample.
    globe = (p(34), p(27), p(194), p(187))
    grid_width = max(1, p(8))
    draw.ellipse(globe, outline=cyan, width=grid_width)
    draw.ellipse((p(72), p(27), p(156), p(187)), outline=cyan, width=max(1, p(6)))
    draw.arc((p(34), p(52), p(194), p(129)), 8, 172, fill=cyan, width=max(1, p(6)))
    draw.arc((p(34), p(87), p(194), p(165)), 188, 352, fill=cyan, width=max(1, p(6)))
    draw.line(((p(36), p(107)), (p(192), p(107))), fill=cyan, width=max(1, p(6)))

    # A navy clearance ring separates the Beatblock sublabel from the globe.
    draw.ellipse((p(137), p(133), p(239), p(235)), fill=navy)
    draw.ellipse((p(155), p(168), p(222), p(235)), fill=white)
    draw.rectangle((p(184), p(153), p(197), p(172)), fill=white)
    draw.arc(
        (p(139), p(131), p(238), p(220)),
        202,
        342,
        fill=white,
        width=max(1, p(15)),
    )
    draw.rectangle((p(176), p(190), p(183), p(216)), fill=navy)
    draw.rectangle((p(198), p(190), p(205), p(216)), fill=navy)

    return image


def png_bytes(image: Image.Image) -> bytes:
    output = io.BytesIO()
    image.save(output, format="PNG", optimize=True)
    return output.getvalue()


def ico_bytes(image: Image.Image) -> bytes:
    output = io.BytesIO()
    image.resize((256, 256), Image.Resampling.LANCZOS).save(
        output,
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
    )
    return output.getvalue()


def expected_assets() -> dict[Path, bytes]:
    installer = draw_installer_icon()
    online = draw_online_icon()
    validate_online_icon(online)
    return {
        ONLINE_PNG: png_bytes(online),
        INSTALLER_PNG: png_bytes(installer.resize((512, 512), Image.Resampling.LANCZOS)),
        INSTALLER_ICO: ico_bytes(installer),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail if checked-in icons differ from the deterministic generator",
    )
    args = parser.parse_args()

    assets = expected_assets()
    if args.check:
        stale = [path for path, expected in assets.items() if not path.is_file() or path.read_bytes() != expected]
        if stale:
            for path in stale:
                print(f"stale or missing icon: {path.relative_to(ROOT)}")
            return 1
        print(f"Validated {len(assets)} generated icon assets.")
        return 0

    for path, contents in assets.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(contents)
        print(f"Wrote {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
