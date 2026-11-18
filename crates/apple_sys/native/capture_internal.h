#ifndef SPLICE_CAPTURE_INTERNAL_H
#define SPLICE_CAPTURE_INTERNAL_H

#include "platform.h"
#import <ScreenCaptureKit/ScreenCaptureKit.h>

SCContentFilter *splice_capture_prepare(const SpliceCaptureTarget *target, SCStreamConfiguration *configuration,
                                        bool *denied, char **error);
void splice_capture_error(NSError *reason, const char *fallback, bool *denied, char **error);

#endif
