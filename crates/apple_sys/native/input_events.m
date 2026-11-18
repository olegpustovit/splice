#include "input_events.h"

#import <Carbon/Carbon.h>
#include <os/lock.h>
#include <string.h>

static bool format_event(CGEventType type, CGEventRef event, CGRect frame, uint32_t owner,
                         SpliceInputClock clock, bool secure, SpliceEvent *out) {
    const uint64_t timestamp = CGEventGetTimestamp(event);
    if (!clock.active || timestamp < clock.first || timestamp < clock.resumed ||
        timestamp - clock.first < clock.paused || CGRectIsEmpty(frame)) return false;
    CGPoint point = CGEventGetLocation(event);
    if (!CGRectContainsPoint(frame, point)) return false;

    SpliceInputPayload payload = {
        .x = (point.x - frame.origin.x) / frame.size.width,
        .y = (point.y - frame.origin.y) / frame.size.height,
    };
    uint32_t kind;
    switch (type) {
        case kCGEventMouseMoved:
            kind = SPLICE_INPUT_MOVE;
            break;
        case kCGEventLeftMouseDown: case kCGEventRightMouseDown: case kCGEventOtherMouseDown:
            kind = SPLICE_INPUT_DOWN;
            break;
        case kCGEventLeftMouseUp: case kCGEventRightMouseUp: case kCGEventOtherMouseUp:
            kind = SPLICE_INPUT_UP;
            break;
        case kCGEventLeftMouseDragged: case kCGEventRightMouseDragged: case kCGEventOtherMouseDragged:
            kind = SPLICE_INPUT_DRAG;
            break;
        case kCGEventScrollWheel:
            kind = SPLICE_INPUT_SCROLL;
            payload.dx = CGEventGetDoubleValueField(event, kCGScrollWheelEventPointDeltaAxis2);
            payload.dy = CGEventGetDoubleValueField(event, kCGScrollWheelEventPointDeltaAxis1);
            break;
        case kCGEventKeyDown: case kCGEventFlagsChanged: {
            int64_t destination = CGEventGetIntegerValueField(event, kCGEventTargetUnixProcessID);
            if (secure || (owner != 0 && destination != owner)) return false;
            kind = type == kCGEventKeyDown ? SPLICE_INPUT_KEY : SPLICE_INPUT_FLAGS;
            payload.code = (uint32_t)CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode);
            payload.modifiers = CGEventGetFlags(event);
            if (type == kCGEventKeyDown && CGEventGetIntegerValueField(event, kCGKeyboardEventAutorepeat) == 0) {
                UniCharCount length = 0;
                CGEventKeyboardGetUnicodeString(event, 8, &length, payload.text);
                payload.text_length = (uint32_t)MIN(length, 8);
                for (uint32_t i = 0; i < payload.text_length; ++i) {
                    UniChar ch = payload.text[i];
                    if (ch < 0x20 || ch == 0x7F || (ch >= 0xF700 && ch <= 0xF8FF)) {
                        payload.text_length = 0;
                        memset(payload.text, 0, sizeof(payload.text));
                        break;
                    }
                }
            } else if (type == kCGEventKeyDown) {
                return false;
            }
            break;
        }
        default:
            return false;
    }
    if (kind == SPLICE_INPUT_DOWN || kind == SPLICE_INPUT_UP || kind == SPLICE_INPUT_DRAG) {
        payload.code = (uint32_t)CGEventGetIntegerValueField(event, kCGMouseEventButtonNumber);
    }
    *out = (SpliceEvent){ .time_ns = timestamp - clock.first - clock.paused, .kind = kind };
    memcpy(out->payload, &payload, sizeof(payload));
    return true;
}

@implementation SpliceInputMonitor {
    SpliceEventRing *_ring;
    SpliceCaptureGeometry _geometry;
    CGRect _frame;
    os_unfair_lock _lock;
    SpliceInputClock _clock;
    BOOL _closed, _disabled, _started, _exited;
    dispatch_semaphore_t _ready, _done;
    CFMachPortRef _tap;
    CFRunLoopRef _loop;
    BOOL _configured;
}

- (instancetype)initWithGeometry:(SpliceCaptureGeometry)geometry error:(char **)error {
    self = [super init];
    if (self == nil) return nil;
    _ring = splice_events_create(8192, NULL, 0, error);
    if (_ring == NULL) return nil;
    _geometry = geometry;
    _frame = CGRectMake(geometry.rect.x, geometry.rect.y, geometry.rect.width, geometry.rect.height);
    _lock = (os_unfair_lock)OS_UNFAIR_LOCK_INIT;
    _ready = dispatch_semaphore_create(0);
    _done = dispatch_semaphore_create(0);
    return self;
}

- (void)dealloc {
    if (_ring != NULL) splice_events_release(_ring);
}

- (void)handle:(CGEventType)type event:(CGEventRef)event {
    os_unfair_lock_lock(&_lock);
    SpliceInputClock clock = _clock;
    BOOL closed = _closed;
    os_unfair_lock_unlock(&_lock);
    if (closed) return;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        if (type == kCGEventTapDisabledByTimeout && CGPreflightListenEventAccess()) {
            CGEventTapEnable(_tap, true);
        } else {
            os_unfair_lock_lock(&_lock);
            _disabled = YES;
            os_unfair_lock_unlock(&_lock);
        }
        return;
    }
    SpliceEvent encoded;
    bool secure = (type == kCGEventKeyDown || type == kCGEventFlagsChanged) && IsSecureEventInputEnabled();
    if (format_event(type, event, _frame, _geometry.owner_pid, clock, secure, &encoded)) {
        splice_events_push(_ring, &encoded);
    }
}

static CGEventRef input_callback(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *info) {
    (void)proxy;
    [(__bridge SpliceInputMonitor *)info handle:type event:event];
    return event;
}

- (void)updateWindow {
    if (_geometry.window_id == 0) return;
    NSArray<NSDictionary *> *windows = CFBridgingRelease(
        CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, _geometry.window_id));
    NSDictionary *window = windows.firstObject;
    NSDictionary *bounds = window[(__bridge NSString *)kCGWindowBounds];
    NSNumber *owner = window[(__bridge NSString *)kCGWindowOwnerPID];
    CGRect frame;
    _frame = bounds != nil && owner.unsignedIntValue == _geometry.owner_pid &&
        CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)bounds, &frame) ? frame : CGRectNull;
}

- (void)runTap {
    CGEventMask mask = CGEventMaskBit(kCGEventMouseMoved) | CGEventMaskBit(kCGEventScrollWheel) |
        CGEventMaskBit(kCGEventLeftMouseDown) | CGEventMaskBit(kCGEventRightMouseDown) | CGEventMaskBit(kCGEventOtherMouseDown) |
        CGEventMaskBit(kCGEventLeftMouseUp) | CGEventMaskBit(kCGEventRightMouseUp) | CGEventMaskBit(kCGEventOtherMouseUp) |
        CGEventMaskBit(kCGEventLeftMouseDragged) | CGEventMaskBit(kCGEventRightMouseDragged) | CGEventMaskBit(kCGEventOtherMouseDragged) |
        CGEventMaskBit(kCGEventKeyDown) | CGEventMaskBit(kCGEventFlagsChanged);
    _tap = CGEventTapCreate(kCGAnnotatedSessionEventTap, kCGTailAppendEventTap, kCGEventTapOptionListenOnly,
                            mask, input_callback, (__bridge void *)self);
    CFRunLoopSourceRef source = _tap == NULL ? NULL : CFMachPortCreateRunLoopSource(NULL, _tap, 0);
    if (source != NULL) {
        CFRunLoopRef run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        os_unfair_lock_lock(&_lock);
        _loop = run_loop;
        _configured = YES;
        os_unfair_lock_unlock(&_lock);
    }
    dispatch_semaphore_signal(_ready);
    if (source != NULL) {
        for (;;) {
            os_unfair_lock_lock(&_lock);
            BOOL closed = _closed;
            os_unfair_lock_unlock(&_lock);
            if (closed) break;
            @autoreleasepool {
                [self updateWindow];
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, false);
            }
        }
        os_unfair_lock_lock(&_lock);
        CFRunLoopRemoveSource(_loop, source, kCFRunLoopDefaultMode);
        _loop = NULL;
        os_unfair_lock_unlock(&_lock);
        CFRelease(source);
    }
    if (_tap != NULL) {
        CFMachPortInvalidate(_tap);
        CFRelease(_tap);
        _tap = NULL;
    }
    os_unfair_lock_lock(&_lock);
    _exited = YES;
    os_unfair_lock_unlock(&_lock);
    dispatch_semaphore_signal(_done);
}

- (BOOL)start {
    _started = YES;
    [NSThread detachNewThreadWithBlock:^{
        @autoreleasepool {
            [self runTap];
        }
    }];
    if (dispatch_semaphore_wait(_ready, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) != 0) {
        [self stop];
        return NO;
    }
    return _configured;
}

- (BOOL)stop {
    os_unfair_lock_lock(&_lock);
    BOOL joined = !_started || _exited;
    _closed = YES;
    if (_loop != NULL) CFRunLoopStop(_loop);
    os_unfair_lock_unlock(&_lock);
    return joined || dispatch_semaphore_wait(_done, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC)) == 0;
}

- (void)updateClock:(SpliceInputClock)clock {
    os_unfair_lock_lock(&_lock);
    _clock = clock;
    os_unfair_lock_unlock(&_lock);
}

- (BOOL)wasDisabled {
    os_unfair_lock_lock(&_lock);
    BOOL disabled = _disabled;
    os_unfair_lock_unlock(&_lock);
    return disabled;
}

- (size_t)read:(SpliceEvent *)out capacity:(size_t)capacity {
    size_t read = splice_events_peek(_ring, out, capacity);
    return splice_events_ack(_ring, read) ? read : 0;
}

- (uint64_t)dropped {
    return splice_events_dropped(_ring);
}

- (SpliceCaptureGeometry)geometry {
    return _geometry;
}
@end

// The fixture constructs synthetic input. It never posts it or starts a tap.
bool splice_input_fixture(SpliceEvent *out) {
    @autoreleasepool {
        CGEventRef event = CGEventCreateKeyboardEvent(NULL, 0, true);
        if (event == NULL) return false;
        UniChar text[] = {0x0061};
        CGEventKeyboardSetUnicodeString(event, 1, text);
        CGEventSetLocation(event, CGPointMake(25, 60));
        CGEventSetIntegerValueField(event, kCGEventTargetUnixProcessID, 123);
        CGEventSetTimestamp(event, 2900000000);
        const SpliceInputClock clock = {1000000000, 800000000, 2600000000, true};
        CGRect rect = CGRectMake(0, 10, 100, 100);
        bool accepted = format_event(kCGEventKeyDown, event, rect, 123, clock, false, out);
        SpliceEvent rejected;
        bool private = !format_event(kCGEventKeyDown, event, rect, 123, clock, true, &rejected) &&
            !format_event(kCGEventKeyDown, event, rect, 456, clock, false, &rejected) &&
            !format_event(kCGEventKeyDown, event, CGRectOffset(rect, 100, 0), 123, clock, false, &rejected);
        CGEventSetTimestamp(event, 2000000000);
        private = private && !format_event(kCGEventKeyDown, event, rect, 123, clock, false, &rejected);
        CFRelease(event);
        return accepted && private;
    }
}
