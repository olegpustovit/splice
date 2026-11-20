// The text caret of the focused element, from Accessibility (PLAN.md T45.4).
// ponytail: unverified on hardware; written against the documented AX API. Upgrade trigger: the
// first Mac run of a recording with Accessibility granted (check the caret samples in events.json).
#import <ApplicationServices/ApplicationServices.h>
#include <math.h>
#include <stdbool.h>

// Writes the caret (or selection) bounds of the focused UI element in global screen points,
// top-left origin, as x, y, width, height. False without the Accessibility permission, without a
// focused text element, or when the element reports no bounds for its selection. Never prompts.
bool splice_ax_caret(double *out) {
    @autoreleasepool {
        if (out == NULL || !AXIsProcessTrusted()) return false;
        AXUIElementRef system = AXUIElementCreateSystemWide();
        if (system == NULL) return false;
        // An unresponsive app must not stall the recorder's poll.
        AXUIElementSetMessagingTimeout(system, 0.05f);
        bool found = false;
        CFTypeRef focused = NULL;
        if (AXUIElementCopyAttributeValue(system, kAXFocusedUIElementAttribute, &focused) == kAXErrorSuccess &&
            focused != NULL) {
            AXUIElementRef element = (AXUIElementRef)focused;
            CFTypeRef range = NULL;
            if (AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute, &range) == kAXErrorSuccess &&
                range != NULL) {
                CFTypeRef bounds = NULL;
                if (AXUIElementCopyParameterizedAttributeValue(element, kAXBoundsForRangeParameterizedAttribute, range,
                                                              &bounds) == kAXErrorSuccess &&
                    bounds != NULL) {
                    CGRect rect = CGRectZero;
                    if (CFGetTypeID(bounds) == AXValueGetTypeID() &&
                        AXValueGetValue((AXValueRef)bounds, kAXValueTypeCGRect, &rect)) {
                        out[0] = rect.origin.x;
                        out[1] = rect.origin.y;
                        out[2] = rect.size.width;
                        out[3] = rect.size.height;
                        found = isfinite(out[0]) && isfinite(out[1]) && isfinite(out[2]) && isfinite(out[3]) &&
                                out[2] >= 0 && out[3] > 0;
                    }
                    CFRelease(bounds);
                }
                CFRelease(range);
            }
            CFRelease(focused);
        }
        CFRelease(system);
        return found;
    }
}
