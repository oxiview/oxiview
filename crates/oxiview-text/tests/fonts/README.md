# Text test font fixtures

Language: **English**

Unmodified Noto Sans and Noto Sans Arabic variable fonts from the Google Fonts repository, pinned to commit `7af0d90b4d532ffa4b041937b560c6df4e7b1e95`. Both are distributed under the SIL Open Font License 1.1; the corresponding license files are included beside the fonts. These fixtures support deterministic font registration, shaping, fallback, wrapping, and rasterization tests without an unlocked display or network access.

- [NotoSans.ttf](NotoSans.ttf): 2049096 bytes; SHA-256 `bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d`; [source](https://raw.githubusercontent.com/google/fonts/7af0d90b4d532ffa4b041937b560c6df4e7b1e95/ofl/notosans/NotoSans%5Bwdth%2Cwght%5D.ttf).
- [NotoSans-OFL.txt](NotoSans-OFL.txt): 4396 bytes; SHA-256 `cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a`; [source](https://raw.githubusercontent.com/google/fonts/7af0d90b4d532ffa4b041937b560c6df4e7b1e95/ofl/notosans/OFL.txt).
- [NotoSansArabic.ttf](NotoSansArabic.ttf): 844676 bytes; SHA-256 `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`; [source](https://raw.githubusercontent.com/google/fonts/7af0d90b4d532ffa4b041937b560c6df4e7b1e95/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf).
- [NotoSansArabic-OFL.txt](NotoSansArabic-OFL.txt): 4382 bytes; SHA-256 `07fc70bfeb985cc1a87a8587d0a0c80bab11c86c9dc3fd95b6f0cb332f983e96`; [source](https://raw.githubusercontent.com/google/fonts/7af0d90b4d532ffa4b041937b560c6df4e7b1e95/ofl/notosansarabic/OFL.txt).

[OxiviewColorFixture.ttf](OxiviewColorFixture.ttf) is an original deterministic
font under the project's [MIT](../../../../LICENSE-MIT) or
[Apache-2.0](../../../../LICENSE-APACHE) license. Its SHA-256 is
`8a1b7eba71bda206ec7ba9952b1d41bbfb38b80aad69d11955ca61579497609e`.
Run `python3 crates/oxiview-text/tests/fonts/generate-colr.py` with
FontTools 4.60.2 to regenerate it from [the source](generate-colr.py).
The generator has no font or image inputs.

| Character | Drawing |
| --- | --- |
| O | Asymmetric TrueType outline |
| L | COLRv1 linear gradient with rotation and an oblique third point |
| M | COLRv1 Multiply over partially overlapping cyan and yellow shapes |
| S | COLRv1 sweep with different colors in all four quadrants |
| R | COLRv1 radial gradient |
| F | COLRv1 foreground palette entry |
| B | Embedded sbix PNG strike |
| V | COLRv0 layer |

The CPU adapter tests reference additional glyph IDs directly. They cover
an alpha gradient with both CPAL and paint alpha, transparent empty lines
(including a source used with SrcIn), one-stop linear, radial and sweep
gradients, Screen over non-gray midtones, and a variable color stop whose
alpha follows the custom ALPH axis. A second palette supports testing a nonzero
record offset for the first palette.

[OxiviewBitmapFixture.ttf](OxiviewBitmapFixture.ttf) is derived by the same
generator under the same license. Its SHA-256 is
`6d53a4b7a72d898bc877056cacf895b5272a4a80fb7ce7748854b3bf25430d33`.
It has no outline tables. `B` and `🐍` share the original sbix PNG,
`M` has a monochrome EBLC/EBDT bitmap, and space has advance and an empty slot in
the monochrome strike.
