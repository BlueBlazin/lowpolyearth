# HUD icon sources

Search, settings, and close use [Lucide](https://github.com/lucide-icons/lucide/tree/a79b2d131dab2bf20cb224bd0937b439a9c4fa99/icons), revision `a79b2d131dab2bf20cb224bd0937b439a9c4fa99`. The unmodified SVG sources and upstream license are bundled here.

Run `node tools/build-hud-icons.cjs` with `sharp` available to regenerate the 96-pixel RGBA PNGs. The generator uses the mockup's 1.7-unit stroke and white ink for runtime tinting. `ui.rs` embeds these images and displays them at 20 logical pixels (16 for close).

- `search.svg` SHA-256: `283d371c2e433817bb9c0c8310caa6c77fa4177c0f4f1168d9c83b97af7389dc`
- `settings.svg` SHA-256: `0ae27fd0f81999229e3127ac96c5b32edfea448e291d509e76212b917551d66b`
- `x.svg` SHA-256: `4a9cdab38fbb96162e7dace28e33f4ca0e49d8963a6162abc3d4691b7d675117`
