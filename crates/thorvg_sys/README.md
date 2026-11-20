# ThorVG bridge

Native Lottie and SVG playback (PLAN.md T45.2). `vendor/` holds ThorVG's sources, unmodified;
`native/shim.cpp` is the only C++ that touches its API, `native/config.h` stands in for the
header meson would generate, and `src/lib.rs` is the safe Rust `open(path) -> Animation`,
`Animation::info() -> {width, height, frames, fps}` and
`Animation::render(frame, [w, h], &mut rgba)` (straight-alpha RGBA8 into a caller-owned buffer).

- Version: ThorVG **v1.1.4** (released 2026-11-19), from
  `https://github.com/thorvg/thorvg/archive/refs/tags/v1.1.4.tar.gz`,
  sha256 `efd8162187992d2ab33851a6da1668f05e2b5e725b307b344d2c1bdff44d7b0d`.
- Vendored: `inc/thorvg.h`, `src/common`, `src/renderer` (core and `cpu_engine`, without
  `tvgSwRasterSimd.cpp`), `src/modules/raw`, `src/modules/svg`, `src/modules/lottie` (with its
  bundled RapidJSON and JerryScript `jerry-core` for expressions; `jerry-port` is only built with
  threads). Not vendored: the GL/WebGPU engines, the PNG/JPEG/WebP/GIF/font/media loaders, the
  GIF saver, the C API binding, tools and tests. `meson.build` files are dropped.
- Licenses: ThorVG MIT (`vendor/LICENSE`); RapidJSON MIT and JerryScript Apache-2.0, each with
  its `LICENSE` beside its sources.
- Configuration (`native/config.h`): CPU engine, SVG and Lottie loaders, Lottie expressions,
  file I/O. No worker threads, partial rendering, SIMD, OpenMP or logging.
- Built by `cc` as C++17 with ThorVG's release flags (`-O2 -DNDEBUG -fno-exceptions -fno-rtti`,
  warnings off). `build.rs` compiles every `.cpp` under `vendor/src` and splits them over
  `NUM_JOBS` threads, since `cc` alone compiles serially.

Without ThorVG's thread support its internal locks are no-ops and JerryScript has one global
context, so a process-wide mutex in `src/lib.rs` serializes every call. Lottie expressions and
text selectors draw from `rand()`; the shim reseeds it before every load and frame, so frame N
renders the same bytes whatever was rendered before it. A warm `render` allocates nothing on
the Rust side; ThorVG's own per-frame scene work is C++ heap and runs once per file when Splice
builds a picture movie (crates/splice_av), never per playback frame.

Not supported (they need the loaders left out above): raster images embedded in a Lottie or
SVG, and text drawn from fonts rather than glyph outlines. Those parts draw nothing.

To upgrade: download the release tarball, check its sha256, copy the directories listed above
over `vendor/` (deleting `meson.build`, `tvgSwRasterSimd.cpp` and `jerry-port`), compare
meson's `config.h` options with `native/config.h`, update this README and run
`cargo t -p thorvg_sys -p splice_mcp`.

## Evidence for the dependency (AGENTS.md)

Measured on the Linux box (20 threads, rust 1.99.1, gcc), 2026-11-20:

- Cold build of the native library (all 236 translation units, after touching
  `native/shim.cpp`): 7.8 s wall dev, 8.4 s release, 35 s CPU either way. Cargo hands the build
  script `NUM_JOBS`, so the Mac mini's `CARGO_BUILD_JOBS=2` builds it on two threads.
- `libsplice_thorvg.a`: 3.2 MB.
