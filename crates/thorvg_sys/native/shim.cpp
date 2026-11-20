// The only code that touches ThorVG's C++ API: one animation on one software canvas.
// The Rust side serializes every call (ThorVG is built without thread support).
#include <cstdlib>
#include <new>

#include "thorvg.h"

using namespace tvg;

struct SpliceTvg {
    Animation *animation;
    SwCanvas *canvas;
    uint32_t *target;
    uint32_t width, height;
};

// Expressions and text selectors draw from rand(): reseeding before each load and frame makes
// frame N the same whichever frames came before it.
static const unsigned SEED = 0x5911CE;

extern "C" bool splice_tvg_init(void) {
    return Initializer::init(0) == Result::Success;
}

// Opens a Lottie or SVG file. Writes its size in pixels, frame count (0 for a still) and
// duration in seconds (0 for a still).
extern "C" SpliceTvg *splice_tvg_open(const char *path, float *width, float *height, float *frames,
                                      float *duration) {
    srand(SEED);
    auto animation = Animation::gen();
    if (animation == nullptr) return nullptr;
    auto picture = animation->picture();
    float w = 0, h = 0;
    if (picture->load(path) != Result::Success || picture->size(&w, &h) != Result::Success || !(w > 0) ||
        !(h > 0)) {
        delete animation;
        return nullptr;
    }
    auto canvas = SwCanvas::gen();
    if (canvas == nullptr || canvas->add(picture) != Result::Success) {
        delete canvas;
        delete animation;
        return nullptr;
    }
    auto handle = new (std::nothrow) SpliceTvg{animation, canvas, nullptr, 0, 0};
    if (handle == nullptr) {
        delete canvas;
        delete animation;
        return nullptr;
    }
    *width = w;
    *height = h;
    *frames = animation->totalFrame();
    *duration = animation->duration();
    return handle;
}

// Draws frame `frame` scaled to `width` x `height` into `pixels` (straight-alpha RGBA8,
// `width` * `height` * 4 bytes), clearing it first.
extern "C" bool splice_tvg_render(SpliceTvg *handle, float frame, uint32_t *pixels, uint32_t width,
                                  uint32_t height) {
    if (handle->target != pixels || handle->width != width || handle->height != height) {
        if (handle->canvas->target(pixels, width, width, height, ColorSpace::ABGR8888S) != Result::Success) {
            handle->target = nullptr;
            return false;
        }
        handle->animation->picture()->size((float)width, (float)height);
        handle->target = pixels;
        handle->width = width;
        handle->height = height;
    }
    srand(SEED);
    // InsufficientCondition means the frame is already current, and NonSupport a still.
    auto moved = handle->animation->frame(frame);
    if (moved != Result::Success && moved != Result::InsufficientCondition && moved != Result::NonSupport) {
        return false;
    }
    return handle->canvas->update() == Result::Success && handle->canvas->draw(true) == Result::Success &&
           handle->canvas->sync() == Result::Success;
}

extern "C" void splice_tvg_close(SpliceTvg *handle) {
    // The canvas releases its reference to the picture before the animation drops its own.
    delete handle->canvas;
    delete handle->animation;
    delete handle;
}
