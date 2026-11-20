// Resizes another app's window before capture (record.start {source:{window}, size}), with
// Accessibility like Recorder's window picker: kAXSizeAttribute keeps the top-left corner.
#include "platform.h"

#import <ApplicationServices/ApplicationServices.h>
#include <math.h>

static bool ax_frame(AXUIElementRef window, CGRect *out) {
    CFTypeRef position = NULL, size = NULL;
    bool ok = false;
    if (AXUIElementCopyAttributeValue(window, kAXPositionAttribute, &position) == kAXErrorSuccess &&
        AXUIElementCopyAttributeValue(window, kAXSizeAttribute, &size) == kAXErrorSuccess && position != NULL &&
        size != NULL && CFGetTypeID(position) == AXValueGetTypeID() && CFGetTypeID(size) == AXValueGetTypeID()) {
        ok = AXValueGetValue((AXValueRef)position, kAXValueTypeCGPoint, &out->origin) &&
             AXValueGetValue((AXValueRef)size, kAXValueTypeCGSize, &out->size);
    }
    if (position != NULL) CFRelease(position);
    if (size != NULL) CFRelease(size);
    return ok;
}

uint32_t splice_window_resize(uint32_t pid, SpliceCaptureRect frame, double width, double height,
                              SpliceCaptureRect *out) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) return 1;
        AXUIElementRef app = AXUIElementCreateApplication((pid_t)pid);
        if (app == NULL) return 2;
        AXUIElementSetMessagingTimeout(app, 1.0f);
        CFTypeRef windows = NULL;
        uint32_t status = 2;
        if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &windows) == kAXErrorSuccess && windows != NULL &&
            CFGetTypeID(windows) == CFArrayGetTypeID()) {
            // ScreenCaptureKit and Accessibility share global top-left points: the window whose
            // frame matches the capture source is the one to resize (titles can repeat).
            for (CFIndex i = 0; i < CFArrayGetCount(windows); i++) {
                AXUIElementRef window = (AXUIElementRef)CFArrayGetValueAtIndex(windows, i);
                CGRect current;
                if (!ax_frame(window, &current) || fabs(current.origin.x - frame.x) >= 2 ||
                    fabs(current.origin.y - frame.y) >= 2 || fabs(current.size.width - frame.width) >= 2 ||
                    fabs(current.size.height - frame.height) >= 2) {
                    continue;
                }
                CGSize size = CGSizeMake(width, height);
                AXValueRef value = AXValueCreate(kAXValueTypeCGSize, &size);
                AXError result = value == NULL ? kAXErrorFailure : AXUIElementSetAttributeValue(window, kAXSizeAttribute, value);
                if (value != NULL) CFRelease(value);
                // Apps clamp to their own limits: report the frame the window really has.
                if (result == kAXErrorSuccess && ax_frame(window, &current)) {
                    *out = (SpliceCaptureRect){current.origin.x, current.origin.y, current.size.width,
                                               current.size.height};
                    status = 0;
                } else {
                    status = 3;
                }
                break;
            }
        }
        if (windows != NULL) CFRelease(windows);
        CFRelease(app);
        return status;
    }
}
