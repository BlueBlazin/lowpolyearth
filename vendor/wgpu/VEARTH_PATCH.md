# Vendored wgpu 30.0.1

Unmodified crates.io `wgpu` 30.0.1 (upstream commit
40f4a34ebaf56f9a046231f54125ad046239d3f3), except for four closures in
`src/backend/webgpu.rs`, each marked `Vearth patch`.

The browser build uses `panic = "unwind"` so generation workers can recover
from a panicking cell. Under unwinding, wasm-bindgen 0.2.129 requires
`UnwindSafe` closures, which wgpu's WebGPU backend does not yet provide. The
patch switches those closures to wasm-bindgen's `*_assert_unwind_safe`
constructors. Native backends are unchanged.

The root `Cargo.toml` applies this copy through `[patch.crates-io]`. Remove
the directory and the patch entry once an upstream release compiles for
`wasm32-unknown-unknown` with `-C panic=unwind`. Verify with:

    diff -ru <cargo registry>/wgpu-30.0.1 vendor/wgpu
