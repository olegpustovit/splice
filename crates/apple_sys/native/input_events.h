#ifndef SPLICE_INPUT_EVENTS_H
#define SPLICE_INPUT_EVENTS_H

#include "capture_internal.h"

typedef enum {
    SPLICE_INPUT_MOVE = 1,
    SPLICE_INPUT_DOWN,
    SPLICE_INPUT_UP,
    SPLICE_INPUT_DRAG,
    SPLICE_INPUT_SCROLL,
    SPLICE_INPUT_KEY,
    SPLICE_INPUT_FLAGS,
    // A captured window's frame changed: code is its index, x/y/dx/dy its global-point frame.
    SPLICE_INPUT_WINDOW,
} SpliceInputKind;

// Window 0 plus up to three more windows of a multi-window take.
#define SPLICE_INPUT_WINDOWS 4

typedef struct {
    double x, y, dx, dy;
    uint64_t modifiers;
    uint32_t code, text_length;
    uint16_t text[8];
} SpliceInputPayload;

typedef struct {
    uint64_t first, paused, resumed;
    bool active;
} SpliceInputClock;

_Static_assert(sizeof(SpliceInputPayload) == sizeof(((SpliceEvent *)0)->payload), "input event ABI");

@interface SpliceInputMonitor : NSObject
- (instancetype)initWithGeometry:(SpliceCaptureGeometry)geometry error:(char **)error;
// The other windows of a multi-window take (before start): pointer events over them are kept,
// normalized to window 0 like the rest, and every window's frame changes are sampled.
- (void)watchWindows:(const SpliceCaptureGeometry *)windows count:(size_t)count;
- (BOOL)start;
- (BOOL)stop;
- (void)updateClock:(SpliceInputClock)clock;
- (BOOL)wasDisabled;
- (SpliceCaptureGeometry)geometry;
- (size_t)read:(SpliceEvent *)out capacity:(size_t)capacity;
- (uint64_t)dropped;
@end

#endif
