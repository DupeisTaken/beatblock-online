"""Pixelmatch-style deterministic PNG comparison for the UI gate."""
from __future__ import annotations
import argparse
from pathlib import Path
from PIL import Image, ImageChops

parser = argparse.ArgumentParser()
parser.add_argument("actual", type=Path)
parser.add_argument("baseline", type=Path)
parser.add_argument("diff", type=Path)
parser.add_argument("--threshold", type=float, default=0.1)
parser.add_argument("--max-changed-percent", type=float, default=0.05)
args = parser.parse_args()

actual = Image.open(args.actual).convert("RGBA")
baseline = Image.open(args.baseline).convert("RGBA")
if actual.size != baseline.size:
    raise SystemExit(f"size mismatch: {actual.size} != {baseline.size}")

# Pixelmatch's threshold is perceptual; this conservative channel-distance
# equivalent gives stable results for this deliberately palette-limited UI.
limit = round(255 * args.threshold)
changed = 0
diff = Image.new("RGBA", actual.size, (0, 0, 0, 0))
ap, bp, dp = actual.load(), baseline.load(), diff.load()
for y in range(actual.height):
    for x in range(actual.width):
        delta = max(abs(ap[x, y][i] - bp[x, y][i]) for i in range(4))
        if delta > limit:
            changed += 1
            dp[x, y] = (255, 0, 64, 255)
        else:
            gray = sum(ap[x, y][:3]) // 9
            dp[x, y] = (gray, gray, gray, 90)
args.diff.parent.mkdir(parents=True, exist_ok=True)
diff.save(args.diff)
percent = changed * 100 / (actual.width * actual.height)
print(f"{args.actual.name}: {changed} pixels ({percent:.5f}%)")
if percent > args.max_changed_percent:
    raise SystemExit(1)
