#!/usr/bin/env python3
"""Builds assets/icons.ttf, the toolbar icon font, from the outlined shapes in tools/icon-src/.

Run:  python3 tools/build_icon_font.py      (needs: pip install fonttools)

Each icon is one glyph on a 1000 x 1000 em box (baseline at the bottom). The app never reads the
SVG files; they are only the source of the glyphs.
"""
import re
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "icon-src"
OUT = ROOT.parent / "assets" / "icons.ttf"

# glyph name -> private use code point (keep in sync with `Icon` in src/overlay/icons.rs)
GLYPHS = [
    ("capture", 0xE000),
    ("undo", 0xE001),
    ("copy", 0xE002),
    ("save", 0xE003),
    ("cancel", 0xE004),
    ("magnet", 0xE005),      # with the bolt: magnet on
    ("magnet_off", 0xE006),  # without it
]

def paths(svg: Path):
    return dict(re.findall(r'<path id="([^"]+)" d="([^"]+)"', svg.read_text()))

def magnet_transform(ds):
    """Fits the magnet artwork (with the bolt) into the 100 box, centred, like the old drawing did.
    Both magnet glyphs use this, so the magnet does not move when the bolt comes and goes."""
    pen = BoundsPen(None)
    for d in ds:
        parse_path(d, pen)
    x0, y0, x1, y1 = pen.bounds
    k = 100.0 / max(x1 - x0, y1 - y0)
    return k, 50 - (x0 + x1) / 2 * k, 50 - (y0 + y1) / 2 * k

def glyph(ds, scale=1.0, dx=0.0, dy=0.0):
    pen = TTGlyphPen(None)
    # 100 box (y down) -> 1000 em (y up)
    m = (10 * scale, 0, 0, -10 * scale, 10 * dx, 1000 - 10 * dy)
    for d in ds:
        parse_path(d, TransformPen(Cu2QuPen(pen, 1.0, reverse_direction=True), m))
    return pen.glyph()

glyphs = {".notdef": TTGlyphPen(None).glyph()}
for name in ("capture", "undo", "copy", "save", "cancel"):
    glyphs[name] = glyph(paths(SRC / f"{name}.svg").values())
mp = paths(SRC / "magnet.svg")
k, dx, dy = magnet_transform([mp["magnet-body"], mp["magnet-bolt"]])
glyphs["magnet"] = glyph([mp["magnet-body"], mp["magnet-bolt"]], k, dx, dy)
glyphs["magnet_off"] = glyph([mp["magnet-body"]], k, dx, dy)

order = [".notdef"] + [n for n, _ in GLYPHS]
fb = FontBuilder(1000, isTTF=True)
fb.setupGlyphOrder(order)
fb.setupCharacterMap({cp: n for n, cp in GLYPHS})
fb.setupGlyf(glyphs)
fb.setupHorizontalMetrics({n: (1000, 0) for n in order})
fb.setupHorizontalHeader(ascent=1000, descent=0)
fb.setupNameTable({"familyName": "Scotshreen Icons", "styleName": "Regular"})
fb.setupOS2(sTypoAscender=1000, sTypoDescender=0, usWinAscent=1000, usWinDescent=0)
fb.setupPost()
fb.save(OUT)
print("wrote", OUT, f"({OUT.stat().st_size} bytes)")
