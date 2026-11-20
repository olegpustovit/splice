// What ThorVG's meson configure would generate for Splice's build (see build.rs and README.md):
// the CPU rasterizer, the SVG and Lottie loaders with expressions, file I/O; no worker
// threads, partial rendering, SIMD, logging or other loaders.
#pragma once
#define THORVG_VERSION_STRING "1.1.4"
#define THORVG_CPU_ENGINE_SUPPORT 1
#define THORVG_SVG_LOADER_SUPPORT 1
#define THORVG_LOTTIE_LOADER_SUPPORT 1
#define THORVG_LOTTIE_EXPRESSIONS_SUPPORT 1
#define THORVG_FILE_IO_SUPPORT 1
#define WIN32_LEAN_AND_MEAN 1
