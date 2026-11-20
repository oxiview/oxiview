# Editing snapshot font

[English](README.md)

`EditingFixture.otf` is a renamed subset of Noto Sans CJK SC Regular for the Chinese text in the deterministic CPU snapshots. It retains the original copyright metadata and the SIL Open Font License. FontTools 4.60.2 retained the glyphs for 山间书屋林笔记中文输入候选. The family and PostScript names identify the subset as Oxiview Editing Fixture. [Upstream font](https://github.com/notofonts/noto-cjk/blob/main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf) · [License](OFL.txt)

The CPU snapshot registers this subset alongside the text crate's pinned Noto Sans and Noto Sans Arabic. The source and preview name these families explicitly when running the test with system font discovery disabled. The application uses the platform font database for fallback at runtime.
