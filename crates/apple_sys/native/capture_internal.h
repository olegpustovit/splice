#ifndef SPLICE_CAPTURE_INTERNAL_H
#define SPLICE_CAPTURE_INTERNAL_H

#include "platform.h"
#import <ScreenCaptureKit/ScreenCaptureKit.h>

typedef struct {
    SpliceCaptureRect rect;
    uint32_t window_id, owner_pid;
} SpliceCaptureGeometry;

SCContentFilter *splice_capture_prepare(const SpliceCaptureTarget *target, SCStreamConfiguration *configuration,
                                        SpliceCaptureGeometry *geometry, bool *denied, char **error);
void splice_capture_error(NSError *reason, const char *fallback, bool *denied, char **error);

#endif
