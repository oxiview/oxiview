# SPDX-License-Identifier: MIT OR Apache-2.0
"""Generate an original fixture with FontTools 4.60.2."""

from pathlib import Path
from struct import pack
from zlib import crc32

from fontTools.colorLib.builder import buildCOLR, buildCPAL
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables.DefaultTable import DefaultTable
from fontTools.ttLib.tables.otTables import CompositeMode, PaintFormat
from fontTools.ttLib.tables.sbixGlyph import Glyph
from fontTools.ttLib.tables.sbixStrike import Strike
from fontTools.varLib.builder import buildVarData, buildVarRegionList, buildVarStore


def contour(*points):
    pen = TTGlyphPen(None)
    if points:
        pen.moveTo(points[0])
        for point in points[1:]:
            pen.lineTo(point)
        pen.closePath()
    return pen.glyph()


def rectangle(x0, y0, x1, y1):
    return contour((x0, y0), (x1, y0), (x1, y1), (x0, y1))


def paint(format, **fields):
    return dict(Format=format, **fields)


def solid(index):
    return paint(PaintFormat.PaintSolid, PaletteIndex=index, Alpha=1)


def clip(name, child):
    return paint(PaintFormat.PaintGlyph, Glyph=name, Paint=child)


def color_line(*stops, extend="pad"):
    return {
        "Extend": extend,
        "ColorStop": [
            dict(StopOffset=offset, PaletteIndex=index, Alpha=alpha[0] if alpha else 1)
            for offset, index, *alpha in stops
        ],
    }


def horizontal_gradient(line):
    return paint(
        PaintFormat.PaintLinearGradient,
        ColorLine=line,
        x0=0, y0=0, x1=800, y1=0, x2=0, y2=800,
    )


def bitmap_png():
    def chunk(tag, data):
        return pack(">I", len(data)) + tag + data + pack(">I", crc32(tag + data))

    # zlib stream with one uncompressed block of four opaque magenta RGBA rows.
    rows = bytes([0, 240, 40, 180, 255] + [240, 40, 180, 255] * 3) * 4
    from zlib import adler32

    stream = (
        b"\x78\x01\x01"
        + pack("<HH", len(rows), len(rows) ^ 0xFFFF)
        + rows
        + pack(">I", adler32(rows))
    )
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", pack(">IIBBBBB", 4, 4, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", stream)
        + chunk(b"IEND", b"")
    )


def save_bitmap_only_fixture(builder):
    font = builder.font
    for tag in ("glyf", "loca", "COLR", "CPAL", "fvar"):
        del font[tag]
    font["maxp"].tableVersion = 0x00005000
    builder.setupCharacterMap({0x20: "space", ord("B"): "bitmap",
                               ord("M"): "outline", 0x1F40D: "bitmap"})
    builder.setupNameTable(
        dict(familyName="Oxiview Bitmap Fixture", styleName="Regular",
             uniqueFontIdentifier="Oxiview Bitmap Fixture 1",
             fullName="Oxiview Bitmap Fixture Regular", psName="OxiviewBitmapFixture-Regular",
             version="Version 1.0")
    )
    # Keep the original sbix PNG. Add one 16 ppem, 1-bit EBDT image with four
    # padded rows and horizontal small glyph metrics (image format 1).
    # The adjacent space has equal start/end offsets and no bitmap data.
    glyph = bytes([4, 4, 0, 4, 16, 0xF0, 0xF0, 0xF0, 0xF0])
    font["EBDT"] = DefaultTable("EBDT")
    font["EBDT"].data = pack(">I", 0x00020000) + glyph
    mono_id = font.getGlyphID("outline")
    space_id = font.getGlyphID("space")
    index = (pack(">HHI", space_id, mono_id, 8) + pack(">HHI", 1, 1, 4)
             + pack(">III", 0, 0, len(glyph)))
    horizontal_metrics = bytes([4, 0, 4, 1, 0, 0, 0, 12, 4, 0, 0, 0])
    font["EBLC"] = DefaultTable("EBLC")
    font["EBLC"].data = (pack(">II", 0x00020000, 1)
                         + pack(">IIII", 56, len(index), 1, 0)
                         + horizontal_metrics + bytes(12)
                         + pack(">HHBBBB", space_id, mono_id, 16, 16, 1, 1) + index)
    builder.save(Path(__file__).with_name("OxiviewBitmapFixture.ttf"))


def main():
    outlines = {
        ".notdef": contour(),
        "space": contour(),
        "outline": contour((0, 0), (700, 0), (0, 650)),
        "linear": contour(),
        "multiply": contour(),
        "sweep": contour(),
        "radial": contour(),
        "foreground": contour(),
        "bitmap": contour(),
        "colr0": contour(),
        "square": rectangle(0, 0, 800, 800),
        "left": rectangle(0, 0, 600, 800),
        "right": rectangle(200, 0, 800, 800),
        "alpha_gradient": contour(),
        "empty_gradient": contour(),
        "one_stop": contour(),
        "screen": contour(),
        "variable": contour(),
        "transparent_source": contour(),
        "one_stop_radial": contour(),
        "one_stop_sweep": contour(),
    }
    builder = FontBuilder(1000, isTTF=True)
    builder.setupGlyphOrder(list(outlines))
    builder.setupCharacterMap(
        {ord(ch): name for ch, name in zip(" OLMSRFBV", list(outlines)[1:10])}
    )
    builder.setupGlyf(outlines)
    builder.setupHorizontalMetrics(
        {name: (1000, 200 if name == "right" else 0) for name in outlines}
    )
    builder.setupHorizontalHeader(ascent=800, descent=-200)
    builder.setupNameTable(
        dict(familyName="Oxiview Color Fixture", styleName="Regular",
             uniqueFontIdentifier="Oxiview Color Fixture 1",
             fullName="Oxiview Color Fixture Regular", psName="OxiviewColorFixture-Regular",
             version="Version 1.0")
    )
    builder.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    builder.setupPost()
    # Only the paint varies. A regular-only outline lets Parley synthesize bold.
    builder.setupFvar([("ALPH", 0, 0, 1, "Paint Alpha")], [])
    region_list = buildVarRegionList([{"ALPH": (0, 1, 1)}], ["ALPH"])
    var_store = buildVarStore(region_list, [buildVarData([0], [[0], [8192]])])

    # The zero-angle ray points right and the positive angle sweeps upward on screen.
    # Hard stops make the four physical quadrants distinguishable without sampling seams.
    square_clip = (0, 0, 800, 800)
    paints = {
        "linear": clip(
            "square",
            paint(
                PaintFormat.PaintRotate,
                angle=90,
                Paint=paint(
                    PaintFormat.PaintLinearGradient,
                    ColorLine=color_line((0, 0), (1, 1)),
                    x0=0, y0=0, x1=800, y1=0, x2=200, y2=800,
                ),
            ),
        ),
        "multiply": paint(
            PaintFormat.PaintComposite,
            SourcePaint=clip("right", solid(3)),
            CompositeMode=CompositeMode.MULTIPLY,
            BackdropPaint=clip("left", solid(2)),
        ),
        "sweep": clip(
            "square",
            paint(
                PaintFormat.PaintSweepGradient,
                ColorLine=color_line((0, 0), (0.25, 0), (0.25, 4), (0.5, 4),
                                     (0.5, 1), (0.75, 1), (0.75, 3), (1, 3)),
                centerX=400, centerY=400, startAngle=0, endAngle=360,
            ),
        ),
        "radial": clip(
            "square",
            paint(
                PaintFormat.PaintRadialGradient,
                ColorLine=color_line((0, 0), (1, 1)),
                x0=400, y0=400, r0=0, x1=400, y1=400, r1=400,
            ),
        ),
        "foreground": clip("square", solid(0xFFFF)),
        "colr0": [("square", 4)],
        "alpha_gradient": clip(
            "square", horizontal_gradient(color_line((0, 5, 0.5), (1, 6)))
        ),
        "empty_gradient": clip("square", horizontal_gradient(color_line())),
        "one_stop": clip(
            "square", horizontal_gradient(color_line((0.625, 5), extend="repeat"))
        ),
        "screen": paint(
            PaintFormat.PaintComposite,
            SourcePaint=clip("right", solid(6)),
            CompositeMode=CompositeMode.SCREEN,
            BackdropPaint=clip("left", solid(7)),
        ),
        "variable": clip(
            "square",
            paint(
                PaintFormat.PaintVarLinearGradient,
                ColorLine={
                    "Extend": "reflect",
                    "ColorStop": [
                        dict(StopOffset=0.35, PaletteIndex=7, Alpha=0.5, VarIndexBase=0)
                    ],
                },
                x0=0, y0=0, x1=800, y1=0, x2=0, y2=800,
                VarIndexBase=0xFFFFFFFF,
            ),
        ),
        "transparent_source": paint(
            PaintFormat.PaintComposite,
            SourcePaint=clip("right", horizontal_gradient(color_line())),
            CompositeMode=CompositeMode.SRC_IN,
            BackdropPaint=clip("left", solid(4)),
        ),
        "one_stop_radial": clip(
            "square",
            paint(
                PaintFormat.PaintRadialGradient,
                ColorLine=color_line((0.25, 7), extend="reflect"),
                x0=0, y0=400, r0=200, x1=800, y1=400, r1=200,
            ),
        ),
        "one_stop_sweep": clip(
            "square",
            paint(
                PaintFormat.PaintSweepGradient,
                ColorLine=color_line((0.35, 7)),
                centerX=400, centerY=400, startAngle=60, endAngle=280,
            ),
        ),
    }
    builder.font["COLR"] = buildCOLR(
        paints,
        glyphMap=builder.font.getReverseGlyphMap(),
        clipBoxes=dict.fromkeys((name for name in paints if name != "colr0"), square_clip),
    )
    builder.font["COLR"].table.VarStore = var_store
    palette = [(1, 0, 0, 1), (0, 0, 1, 1), (0, 1, 1, 1), (1, 1, 0, 1), (0, 1, 0, 1)]
    palette += [tuple(byte / 255 for byte in color) for color in [
        (128, 30, 90, 128), (65, 150, 185, 255), (128, 30, 90, 255),
    ]]
    builder.font["CPAL"] = buildCPAL([palette, [(1, 0, 1, 1)] * len(palette)])

    sbix = newTable("sbix")
    strike = Strike(ppem=16)
    strike.glyphs["bitmap"] = Glyph(glyphName="bitmap", graphicType="png ", imageData=bitmap_png())
    sbix.strikes[16] = strike
    builder.font["sbix"] = sbix
    builder.font["head"].created = 2082844800
    builder.font["head"].modified = 2082844800
    builder.font.recalcTimestamp = False
    builder.save(Path(__file__).with_name("OxiviewColorFixture.ttf"))
    save_bitmap_only_fixture(builder)


if __name__ == "__main__":
    main()
