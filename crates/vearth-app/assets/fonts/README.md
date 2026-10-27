# Bundled UI fonts

## IBM Plex

These font files come from IBM's official
[`IBM/plex`](https://github.com/IBM/plex) repository at commit
`2f9ba1b25957d958db71a849e85d72e3ecfb845a`.

- `IBMPlexSans-Regular.ttf`
- `IBMPlexSans-Medium.ttf`
- `IBMPlexMono-Regular.ttf`

SHA-256, in the same order:

- `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5`
- `331c8639d7598b2cde62a911a71db195e30cb655cd6bdf2e324a7e984955f907`
- `7c6fbddca4b700be918f5f6183d9bd4464fa427fe435f0b480d77fe2bb8c5a43`

They are distributed under the
[SIL Open Font License 1.1](https://openfontlicense.org/open-font-license-official-text/).
The license text is stored as `OFL.txt` in this directory and packaged as
`OFL-IBMPlex.txt`.

## Noto Sans CJK

`NotoSansCJKjp-Regular.otf` comes from the official
[`notofonts/noto-cjk`](https://github.com/notofonts/noto-cjk) repository,
release `Sans2.004`.

SHA-256: `68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5`.

It is used only as a fallback when IBM Plex does not contain a glyph, giving
the shared UI font families coverage for Chinese, Japanese, and Korean place
names. It is distributed under the
[SIL Open Font License 1.1](https://openfontlicense.org/open-font-license-official-text/).
The license text is stored and packaged as `OFL-NotoSansCJK.txt`.

## Arrival-title fonts

The `titles/manifest.json` pins the unmodified, full TTF files and each family's
SIL Open Font License to Google Fonts repository commit
`809e4d8b8d7e9364a914909bb777679606c178b8`. These are not web text subsets.
Variable fonts are styled at runtime; the distributed font bytes are unchanged.
The dedicated arrival families do not replace the shared IBM Plex UI fonts.

Run `python3 tools/verify-title-fonts.py` to verify every font, license hash,
and renderer reference; add `--fetch` to restore exact pinned upstream files.
All `titles/*-OFL.txt` files are distributed in `Resources/TitleFontLicenses/`.
The manifest records upstream paths, revision, and SHA-256 for every asset.
