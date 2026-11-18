#ifndef SPLICE_CAPTURE_INTERNAL_H
#define SPLICE_CAPTURE_INTERNAL_H

#include "platform.h"
#import <ScreenCaptureKit/ScreenCaptureKit.h>

typedef struct {
    SpliceCaptureRect rect;
    double scale;
    uint32_t kind, window_id, owner_pid;
} SpliceCaptureGeometry;

SCContentFilter *splice_capture_prepare(const SpliceCaptureTarget *target, SCStreamConfiguration *configuration,
                                        SpliceCaptureGeometry *geometry, bool *denied, char **error);
void splice_capture_error(NSError *reason, const char *fallback, bool *denied, char **error);

#endif
