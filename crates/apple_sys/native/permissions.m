#include "platform.h"

#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreGraphics/CoreGraphics.h>

static uint32_t media_status(AVMediaType media) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:media]) {
        case AVAuthorizationStatusAuthorized: return SPLICE_PERMISSION_GRANTED;
        case AVAuthorizationStatusNotDetermined: return SPLICE_PERMISSION_NOT_DETERMINED;
        case AVAuthorizationStatusDenied: return SPLICE_PERMISSION_DENIED;
        case AVAuthorizationStatusRestricted: return SPLICE_PERMISSION_RESTRICTED;
    }
    return SPLICE_PERMISSION_NOT_GRANTED;
}

uint32_t splice_permission_status(uint32_t permission) {
    @autoreleasepool {
        switch (permission) {
            case SPLICE_PERMISSION_SCREEN:
                return CGPreflightScreenCaptureAccess() ? SPLICE_PERMISSION_GRANTED : SPLICE_PERMISSION_NOT_GRANTED;
            case SPLICE_PERMISSION_MICROPHONE:
                return media_status(AVMediaTypeAudio);
            case SPLICE_PERMISSION_CAMERA:
                return media_status(AVMediaTypeVideo);
            case SPLICE_PERMISSION_INPUT:
                return CGPreflightListenEventAccess() ? SPLICE_PERMISSION_GRANTED : SPLICE_PERMISSION_NOT_GRANTED;
            case SPLICE_PERMISSION_ACCESSIBILITY:
                return AXIsProcessTrusted() ? SPLICE_PERMISSION_GRANTED : SPLICE_PERMISSION_NOT_GRANTED;
            default:
                return SPLICE_PERMISSION_NOT_GRANTED;
        }
    }
}

bool splice_permission_request(uint32_t permission, bool open_settings) {
    if (![NSThread isMainThread] || permission > SPLICE_PERMISSION_ACCESSIBILITY) return false;
    @autoreleasepool {
        if (splice_permission_status(permission) == SPLICE_PERMISSION_GRANTED) return true;
        static uint32_t prompted;
        uint32_t bit = 1U << permission;
        if ((prompted & bit) == 0) {
            prompted |= bit;
            if (permission == SPLICE_PERMISSION_ACCESSIBILITY) {
                // Adds Splice to the Accessibility list with the system prompt.
                AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef) @{(__bridge NSString *)kAXTrustedCheckOptionPrompt: @YES});
            } else if (permission == SPLICE_PERMISSION_SCREEN || permission == SPLICE_PERMISSION_INPUT) {
                dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
                    if (permission == SPLICE_PERMISSION_SCREEN) {
                        CGRequestScreenCaptureAccess();
                    } else {
                        CGRequestListenEventAccess();
                    }
                });
            } else {
                AVMediaType media = permission == SPLICE_PERMISSION_MICROPHONE ? AVMediaTypeAudio : AVMediaTypeVideo;
                if (media_status(media) == SPLICE_PERMISSION_NOT_DETERMINED) {
                    [AVCaptureDevice requestAccessForMediaType:media completionHandler:^(BOOL granted) {
                        (void)granted;
                    }];
                }
            }
        }
        if (!open_settings) return true;
        NSArray<NSString *> *panes = @[@"Privacy_ScreenCapture", @"Privacy_Microphone",
                                        @"Privacy_Camera", @"Privacy_ListenEvent", @"Privacy_Accessibility"];
        NSString *uri = [@"x-apple.systempreferences:com.apple.preference.security?" stringByAppendingString:panes[permission]];
        // This URL scheme is the Settings deep link. The CLI also prints the pane's name.
        [[NSWorkspace sharedWorkspace] openURL:[NSURL URLWithString:uri]
                                  configuration:[NSWorkspaceOpenConfiguration configuration]
                              completionHandler:nil];
        return true;
    }
}
