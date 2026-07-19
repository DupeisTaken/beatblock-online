"""Generate the checked-in Beatblock Online icon assets.

The in-game mark pairs a heavy internet globe with Cranky and the gameplay
paddle. It follows Beatblock's native 72 px menu pipeline: hard pixel edges,
black forms, white contrast keylines, and a transparent canvas. The installer
version adds color and anti-aliasing for Windows shell sizes.
"""

from __future__ import annotations

import argparse
import io
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parents[1]
ONLINE_PNG = ROOT / "mod" / "shared" / "assets" / "online.png"
INSTALLER_PNG = ROOT / "companion" / "assets" / "installer.png"
INSTALLER_ICO = ROOT / "companion" / "assets" / "installer.ico"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)

# Native-size trace of the supplied globe template. Keeping the silhouette as a
# readable one-bit mask preserves its exact four-column, three-band construction
# without checking in the source image's baked checkerboard background.
GLOBE_TEMPLATE = """
......................##########......................
..................#################...................
................######################................
..............##########################..............
............############.####.############............
...........############..####...###########...........
..........#####..#####...####....####.######..........
........######...####....####.....####..#####.........
.......######...####.....####.....####...#####........
.......####.....####.....####......####....####.......
......####.....####......####......####.....####......
.....######....####......####.......####...######.....
....##############.......####.......##############....
....#################....####....#################....
...####..###################################..#####...
...####......#############################.....####...
..####.......############################.......####..
..####.......####........####.........####......####..
.####........###.........####.........####.......####.
.####........###.........####.........####.......####.
.####........###.........####.........####........###.
.###........####.........####..........###........###.
####........####.........####..........###........####
####........####.........####..........###........####
######################################################
######################################################
######################################################
##############################......##################
####........####.........####..........###........####
####........####.........####..........###........####
####........####.........###...........###........####
####........####.........###...........###........####
.###........####.........####..........###........###.
.###........####.........####.........####........###.
.####........###.........####.........####.......####.
.####........###.........####.........####.......####.
..####.......####........####.........####......####..
..####.......####.##################.####.......####..
...####......#############################.....####...
...####..####################################..####...
....##################...####...##################....
....##############.......####.......##############....
.....######....####......####.......####..#######.....
......####.....####......####.......###.....####......
......#####.....####.....####......####....#####......
.......#####....####.....####.....####...######.......
........######...####....####.....####..######........
.........######..#####...####....####..######.........
...........############..####...###########...........
............############.####..###########............
..............##########################..............
................######################................
..................##################..................
.....................###########......................
""".strip().splitlines()


def draw_online_icon() -> Image.Image:
    """Draw the 72 px globe-and-Cranky menu sprite."""

    image = Image.new("RGBA", (72, 72), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    black = (0, 0, 0, 255)
    white = (255, 255, 255, 255)

    # Stamp the exact supplied silhouette before adding the native white safety
    # edge. The mask is offset two pixels to leave room for that edge.
    globe_mask = Image.new("L", image.size, 0)
    globe_draw = ImageDraw.Draw(globe_mask)
    if len(GLOBE_TEMPLATE) != 54 or any(len(row) != 54 for row in GLOBE_TEMPLATE):
        raise ValueError("Globe template must remain an exact 54x54 mask")
    for y, row in enumerate(GLOBE_TEMPLATE, start=2):
        for x, pixel in enumerate(row, start=2):
            if pixel == "#":
                globe_draw.point((x, y), fill=255)
    image.paste(white, mask=globe_mask.filter(ImageFilter.MaxFilter(5)))
    image.paste(black, mask=globe_mask)

    # Player.lua constructs Cranky from a circular body plus a triangular handle
    # and an annular paddle. Reusing those same primitives makes the sublabel
    # immediately recognizable without coupling this static menu image to a
    # live Player instance.
    center = (48, 49)
    body_bounds = (37, 38, 59, 60)
    paddle_angle = 27
    paddle_half_angle = 28
    paddle_inner_radius = 15
    paddle_outer_radius = 21

    def point(radius: float, angle: float) -> tuple[int, int]:
        radians = math.radians(angle)
        return (
            round(center[0] + math.cos(radians) * radius),
            round(center[1] + math.sin(radians) * radius),
        )

    outer_angles = range(
        paddle_angle - paddle_half_angle,
        paddle_angle + paddle_half_angle + 1,
        4,
    )
    inner_angles = range(
        paddle_angle + paddle_half_angle,
        paddle_angle - paddle_half_angle - 1,
        -4,
    )
    paddle = tuple(point(paddle_outer_radius, angle) for angle in outer_angles) + tuple(
        point(paddle_inner_radius, angle) for angle in inner_angles
    )
    handle = (
        point(7, paddle_angle - 9),
        point(paddle_inner_radius + 1, paddle_angle - 6),
        point(paddle_inner_radius + 1, paddle_angle + 6),
        point(7, paddle_angle + 9),
    )

    # A single outside keyline keeps the composed mascot legible against the
    # menu's alternating pale background and heavy black radial track.
    cranky_mask = Image.new("L", image.size, 0)
    cranky_draw = ImageDraw.Draw(cranky_mask)
    cranky_draw.polygon(paddle, fill=255)
    cranky_draw.polygon(handle, fill=255)
    cranky_draw.ellipse(body_bounds, fill=255)
    image.paste(white, mask=cranky_mask.filter(ImageFilter.MaxFilter(5)))

    draw.polygon(paddle, fill=white, outline=black, width=3)
    draw.polygon(handle, fill=white, outline=black, width=3)
    draw.ellipse(body_bounds, fill=white, outline=black, width=3)
    draw.line(((44, 46), (44, 50)), fill=black, width=2)
    draw.line(((51, 46), (51, 50)), fill=black, width=2)
    return image


def validate_online_icon(image: Image.Image) -> None:
    """Guard the contrast contract that prevents another invisible menu icon."""

    if image.size != (72, 72) or image.mode != "RGBA":
        raise ValueError("Online icon must remain a 72x72 RGBA image")
    pixels = list(image.get_flattened_data())
    black_pixels = sum(pixel == (0, 0, 0, 255) for pixel in pixels)
    white_pixels = sum(pixel == (255, 255, 255, 255) for pixel in pixels)
    transparent_pixels = sum(pixel[3] == 0 for pixel in pixels)
    if black_pixels < 650 or white_pixels < 300:
        raise ValueError("Online icon must retain substantial black keylines and white forms")
    if transparent_pixels < 1_500:
        raise ValueError("Online icon needs transparent breathing room around the silhouette")

    # These samples protect the meaning of the sprite, not just its dimensions:
    # globe core at top-left, Cranky's two eyes, and paddle at bottom-right.
    required_black_pixels = ((29, 4), (44, 47), (51, 47), (66, 58))
    if any(image.getpixel(point) != (0, 0, 0, 255) for point in required_black_pixels):
        raise ValueError("Online icon must retain its globe, Cranky face, and paddle landmarks")


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
