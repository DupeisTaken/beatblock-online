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
GLOBE_TEMPLATE_SOURCE = ROOT / "scripts" / "assets" / "globe-template.png"
MENU_GLOBE_TEMPLATE_SOURCE = ROOT / "scripts" / "assets" / "globe-template-menu.png"
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)


def globe_template_mask(size: int) -> Image.Image:
    """Load the cleaned supplied globe silhouette at a requested square size."""

    source = Image.open(GLOBE_TEMPLATE_SOURCE).convert("RGBA")
    alpha = source.getchannel("A")
    if source.size != (452, 452) or alpha.getbbox() != (0, 0, 452, 452):
        raise ValueError("Globe template source must remain the cleaned 452x452 silhouette")
    return alpha.resize((size, size), Image.Resampling.LANCZOS)


def menu_globe_template_mask() -> Image.Image:
    """Load the fixed native-size trace without resampling the approved sprite."""

    mask = Image.open(MENU_GLOBE_TEMPLATE_SOURCE).convert("L")
    if mask.size != (54, 54) or mask.getbbox() != (0, 0, 54, 54):
        raise ValueError("Menu globe template must remain the exact 54x54 trace")
    return mask


def draw_online_icon() -> Image.Image:
    """Draw the 72 px globe-and-Cranky menu sprite."""

    image = Image.new("RGBA", (72, 72), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    black = (0, 0, 0, 255)
    white = (255, 255, 255, 255)

    # Stamp the exact supplied silhouette before adding the native white safety
    # edge. The mask is offset two pixels to leave room for that edge.
    globe_mask = Image.new("L", image.size, 0)
    globe_mask.paste(menu_globe_template_mask(), (2, 2))
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
    """Draw the colored installer mark from the same globe-and-Cranky system."""

    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    unit = size / 256

    def p(value: float) -> int:
        return round(value * unit)

    def odd_px(value: float) -> int:
        pixels = max(3, p(value))
        return pixels if pixels % 2 else pixels + 1

    # The deep circular tile stays distinct against light and dark Windows
    # themes, while transparent corners keep the application mark lightweight.
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

    # Use the actual supplied globe silhouette rather than rebuilding its grid
    # from generic ellipses. The cyan mask is the only foreground geometry here.
    globe_size = p(170)
    globe_mask = globe_template_mask(globe_size)
    # Center the globe independently on the 256-unit circular application tile;
    # Cranky remains an overlapping lower-right badge rather than shifting it.
    globe_position = (p((256 - 170) / 2), p((256 - 170) / 2))
    image.paste(cyan, globe_position, globe_mask)

    # Match Player.lua's construction at installer resolution: circular Cranky,
    # a short triangular handle, and a curved annular paddle sector.
    center = (p(181), p(186))
    paddle_angle = 34
    paddle_half_angle = 28

    def point(radius: float, angle: float) -> tuple[int, int]:
        radians = math.radians(angle)
        return (
            round(center[0] + math.cos(radians) * p(radius)),
            round(center[1] + math.sin(radians) * p(radius)),
        )

    outer_angles = range(
        paddle_angle - paddle_half_angle,
        paddle_angle + paddle_half_angle + 1,
        3,
    )
    inner_angles = range(
        paddle_angle + paddle_half_angle,
        paddle_angle - paddle_half_angle - 1,
        -3,
    )
    paddle = tuple(point(55, angle) for angle in outer_angles) + tuple(
        point(42, angle) for angle in inner_angles
    )
    handle = (
        point(21, paddle_angle - 14),
        point(44, paddle_angle - 9),
        point(44, paddle_angle + 9),
        point(21, paddle_angle + 14),
    )
    body_bounds = (p(152), p(157), p(210), p(215))

    # The navy clearance fuses the three pieces into one badge and prevents the
    # cyan globe from leaking through their internal white areas.
    cranky_mask = Image.new("L", image.size, 0)
    cranky_draw = ImageDraw.Draw(cranky_mask)
    cranky_draw.polygon(paddle, fill=255)
    cranky_draw.polygon(handle, fill=255)
    cranky_draw.ellipse(body_bounds, fill=255)
    image.paste(navy, mask=cranky_mask.filter(ImageFilter.MaxFilter(odd_px(7))))

    draw.polygon(paddle, fill=white, outline=navy, width=max(1, p(5)))
    draw.polygon(handle, fill=white, outline=navy, width=max(1, p(4)))
    draw.ellipse(body_bounds, fill=white, outline=navy, width=max(1, p(5)))
    draw.line(
        ((p(173), p(177)), (p(173), p(194))),
        fill=navy,
        width=max(1, p(5)),
    )
    draw.line(
        ((p(190), p(177)), (p(190), p(194))),
        fill=navy,
        width=max(1, p(5)),
    )

    return image


def validate_installer_icon(image: Image.Image) -> None:
    """Guard the installer mark's template, palette, and mascot landmarks."""

    if image.size != (1024, 1024) or image.mode != "RGBA":
        raise ValueError("Installer working icon must remain a 1024x1024 RGBA image")
    pixels = list(image.get_flattened_data())
    cyan_pixels = sum(pixel == (76, 225, 245, 255) for pixel in pixels)
    white_pixels = sum(pixel == (244, 248, 252, 255) for pixel in pixels)
    transparent_pixels = sum(pixel[3] == 0 for pixel in pixels)
    if cyan_pixels < 150_000 or white_pixels < 15_000:
        raise ValueError("Installer icon must retain its cyan globe and white Cranky badge")
    if transparent_pixels < 300_000:
        raise ValueError("Installer icon needs transparent corners around its circular tile")

    required_pixels = {
        (512, 512): (76, 225, 245, 255),  # centered exact globe bar
        (724, 744): (244, 248, 252, 255),  # Cranky body
        (692, 740): (15, 21, 36, 255),  # Cranky's left eye
        (876, 864): (244, 248, 252, 255),  # paddle
        (0, 0): (0, 0, 0, 0),  # transparent shell corner
    }
    if any(image.getpixel(point) != expected for point, expected in required_pixels.items()):
        raise ValueError("Installer icon must retain its globe, Cranky, and paddle landmarks")


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
    validate_installer_icon(installer)
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
