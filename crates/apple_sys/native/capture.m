#include "capture_internal.h"

#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

void splice_capture_error(NSError *reason, const char *fallback, bool *denied, char **error) {
    if ([reason.domain isEqualToString:SCStreamErrorDomain] && reason.code == SCStreamErrorUserDeclined) {
        *denied = true;
    }
    splice_set_error(error, reason.localizedDescription.UTF8String ?: fallback);
}

static SCShareableContent *shareable_content(bool *denied, char **error) {
    if (!CGPreflightScreenCaptureAccess()) {
        *denied = true;
        splice_set_error(error, "Screen & System Audio Recording permission is unavailable.");
        return nil;
    }
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    __block SCShareableContent *content;
    __block NSError *failure;
    [SCShareableContent getShareableContentExcludingDesktopWindows:YES
                                             onScreenWindowsOnly:YES
                                              completionHandler:^(SCShareableContent *value, NSError *reason) {
        content = value;
        failure = reason;
        dispatch_semaphore_signal(ready);
    }];
    if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
        splice_set_error(error, "ScreenCaptureKit did not respond to source discovery.");
        return nil;
    }
    if (content == nil) splice_capture_error(failure, "Screen capture sources are unavailable.", denied, error);
    return content;
}

static SpliceCaptureRect capture_rect(CGRect rect) {
    return (SpliceCaptureRect){rect.origin.x, rect.origin.y, rect.size.width, rect.size.height};
}

static SpliceCaptureSource display_source(CGDirectDisplayID id, CGRect frame) {
    return (SpliceCaptureSource){
        .rect = capture_rect(frame), .kind = SPLICE_CAPTURE_DISPLAY, .id = id,
        .main_display = id == CGMainDisplayID(),
    };
}

static bool is_splice(SCRunningApplication *app) {
    return app.processID == getpid() || [app.bundleIdentifier isEqualToString:@"com.olegpustovit.splice"];
}

static bool source_name(char **out, NSString *value) {
    if (value.length == 0) return true;
    const char *bytes = value.UTF8String;
    *out = bytes == NULL ? NULL : strdup(bytes);
    return *out != NULL;
}

static bool display_sources(SpliceCaptureSources *out, char **error) {
    uint32_t count = 0;
    if (CGGetActiveDisplayList(0, NULL, &count) != kCGErrorSuccess) {
        splice_set_error(error, "Could not count active displays.");
        return false;
    }
    if (count == 0) return true;
    CGDirectDisplayID *ids = calloc(count, sizeof(*ids));
    out->items = calloc(count, sizeof(*out->items));
    if (ids == NULL || out->items == NULL ||
        CGGetActiveDisplayList(count, ids, &count) != kCGErrorSuccess) {
        free(ids);
        splice_set_error(error, "Could not list active displays.");
        return false;
    }
    for (uint32_t i = 0; i < count; i++) {
        out->items[out->count++] = display_source(ids[i], CGDisplayBounds(ids[i]));
    }
    free(ids);
    return true;
}

bool splice_capture_sources(bool include_windows, SpliceCaptureSources *out, bool *denied, char **error) {
    @autoreleasepool {
        if (!include_windows) return display_sources(out, error);
        SCShareableContent *content = shareable_content(denied, error);
        if (content == nil) return false;
        size_t capacity = content.displays.count + content.windows.count + content.applications.count;
        if (capacity == 0) return true;
        out->items = calloc(capacity, sizeof(*out->items));
        if (out->items == NULL) {
            splice_set_error(error, "Could not allocate capture sources.");
            return false;
        }
        for (SCDisplay *display in content.displays) {
            out->items[out->count++] = display_source(display.displayID, display.frame);
        }
        for (SCWindow *window in content.windows) {
            SCRunningApplication *app = window.owningApplication;
            if (app == nil || is_splice(app) || CGRectIsEmpty(window.frame)) continue;
            SpliceCaptureSource *item = &out->items[out->count++];
            *item = (SpliceCaptureSource){
                .kind = SPLICE_CAPTURE_WINDOW, .id = window.windowID, .owner_pid = (uint32_t)app.processID,
                .rect = capture_rect(window.frame),
            };
            if (!source_name(&item->app, app.bundleIdentifier) || !source_name(&item->name, window.title)) {
                splice_set_error(error, "Could not copy a capture window name.");
                return false;
            }
        }
        for (SCRunningApplication *app in content.applications) {
            if (is_splice(app)) continue;
            SpliceCaptureSource *item = &out->items[out->count++];
            *item = (SpliceCaptureSource){.kind = SPLICE_CAPTURE_APPLICATION, .id = (uint32_t)app.processID};
            if (!source_name(&item->app, app.bundleIdentifier) || !source_name(&item->name, app.applicationName)) {
                splice_set_error(error, "Could not copy a capture application name.");
                return false;
            }
        }
        return true;
    }
}

void splice_capture_sources_dispose(SpliceCaptureSources *sources) {
    for (size_t i = 0; i < sources->count; i++) {
        free(sources->items[i].app);
        free(sources->items[i].name);
    }
    free(sources->items);
}

static SCContentFilter *capture_filter(const SpliceCaptureTarget *target, SCShareableContent *content,
                                       SCStreamConfiguration *configuration, SpliceCaptureGeometry *geometry,
                                       char **error) {
    SCContentFilter *filter = nil;
    CGRect frame = CGRectZero;
    uint32_t owner = 0;
    if (target->kind == SPLICE_CAPTURE_WINDOW) {
        for (SCWindow *window in content.windows) {
            if (window.windowID == target->id && window.owningApplication != nil &&
                !is_splice(window.owningApplication)) {
                filter = [[SCContentFilter alloc] initWithDesktopIndependentWindow:window];
                frame = window.frame;
                owner = (uint32_t)window.owningApplication.processID;
                break;
            }
        }
    } else {
        const uint32_t id = target->id == 0 ? CGMainDisplayID() : target->id;
        NSMutableArray<SCRunningApplication *> *excluded = [NSMutableArray array];
        for (SCRunningApplication *app in content.applications) {
            if (is_splice(app)) [excluded addObject:app];
        }
        for (SCDisplay *display in content.displays) {
            if (display.displayID == id) {
                filter = [[SCContentFilter alloc] initWithDisplay:display excludingApplications:excluded exceptingWindows:@[]];
                frame = display.frame;
                break;
            }
        }
    }
    if (filter == nil) {
        splice_set_error(error, "The selected capture source is no longer available.");
        return nil;
    }
    CGSize bounds = filter.contentRect.size;
    if (target->kind == SPLICE_CAPTURE_AREA) {
        CGRect region = CGRectMake(target->rect.x, target->rect.y, target->rect.width, target->rect.height);
        if (CGRectIsEmpty(region) || !CGRectContainsRect(CGRectMake(0, 0, bounds.width, bounds.height), region)) {
            splice_set_error(error, "The capture area must fit within the selected display.");
            return nil;
        }
        configuration.sourceRect = region;
        frame = CGRectOffset(region, frame.origin.x, frame.origin.y);
        bounds = region.size;
    }
    double width = round(bounds.width * filter.pointPixelScale);
    double height = round(bounds.height * filter.pointPixelScale);
    if (!isfinite(width) || !isfinite(height) || width < 2 || height < 2 || width > 16384 || height > 16384) {
        splice_set_error(error, "Capture dimensions must be between 2 and 16384 pixels.");
        return nil;
    }
    configuration.width = (size_t)width;
    configuration.height = (size_t)height;
    configuration.showsCursor = NO;
    if (geometry != NULL) {
        *geometry = (SpliceCaptureGeometry){
            .rect = capture_rect(frame), .scale = filter.pointPixelScale, .kind = target->kind,
            .window_id = target->kind == SPLICE_CAPTURE_WINDOW ? target->id : 0,
            .owner_pid = owner,
        };
    }
    return filter;
}

SCContentFilter *splice_capture_prepare(const SpliceCaptureTarget *target, SCStreamConfiguration *configuration,
                                        SpliceCaptureGeometry *geometry, bool *denied, char **error) {
    SCShareableContent *content = shareable_content(denied, error);
    return content == nil ? nil : capture_filter(target, content, configuration, geometry, error);
}

void *splice_capture_screenshot(const SpliceCaptureTarget *target, const uint8_t **data, size_t *size,
                                bool *denied, char **error) {
    @autoreleasepool {
        SCStreamConfiguration *configuration = [SCStreamConfiguration new];
        SCContentFilter *filter = splice_capture_prepare(target, configuration, NULL, denied, error);
        if (filter == nil) return NULL;
        dispatch_semaphore_t ready = dispatch_semaphore_create(0);
        __block NSData *bytes;
        __block NSError *failure;
        [SCScreenshotManager captureImageWithFilter:filter configuration:configuration
                                  completionHandler:^(CGImageRef image, NSError *reason) {
            failure = reason;
            if (image != NULL) {
                NSMutableData *encoded = [NSMutableData data];
                CGImageDestinationRef png = CGImageDestinationCreateWithData(
                    (__bridge CFMutableDataRef)encoded, CFSTR("public.png"), 1, NULL);
                if (png != NULL) {
                    CGImageDestinationAddImage(png, image, NULL);
                    if (CGImageDestinationFinalize(png)) bytes = encoded;
                    CFRelease(png);
                }
            }
            dispatch_semaphore_signal(ready);
        }];
        if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
            splice_set_error(error, "ScreenCaptureKit did not respond to the screenshot request.");
            return NULL;
        }
        if (bytes.length == 0) {
            splice_capture_error(failure, "Could not encode a screenshot.", denied, error);
            return NULL;
        }
        *data = bytes.bytes;
        *size = bytes.length;
        return (__bridge_retained void *)bytes;
    }
}
