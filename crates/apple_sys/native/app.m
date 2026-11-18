#include "platform.h"

#import <AppKit/AppKit.h>
#include <pwd.h>
#include <string.h>
#include <unistd.h>

void splice_app_cancel_open_panel(void) {
    if (![NSThread isMainThread]) return;
    @autoreleasepool {
        for (NSWindow *window in NSApp.windows) {
            NSWindow *sheet = window.attachedSheet;
            if ([sheet isKindOfClass:[NSOpenPanel class]]) {
                [(NSOpenPanel *)sheet cancel:nil];
                return;
            }
        }
    }
}

static bool accessory_quit;

@interface SpliceAccessoryDelegate : NSObject <NSApplicationDelegate>
@end

@implementation SpliceAccessoryDelegate
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)sender {
    (void)sender;
    accessory_quit = true;
    return NSTerminateCancel;
}
@end

bool splice_app_start_accessory(void) {
    if (![NSThread isMainThread]) return false;
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        if (![app setActivationPolicy:NSApplicationActivationPolicyAccessory]) return false;
        static SpliceAccessoryDelegate *delegate;
        delegate = [SpliceAccessoryDelegate new];
        accessory_quit = false;
        app.delegate = delegate;
        [app finishLaunching];
        return true;
    }
}

bool splice_app_wait_accessory(double seconds) {
    if (![NSThread isMainThread]) return false;
    @autoreleasepool {
        NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                untilDate:[NSDate dateWithTimeIntervalSinceNow:seconds]
                inMode:NSDefaultRunLoopMode dequeue:YES];
        if (event != nil) [NSApp sendEvent:event];
        return !accessory_quit;
    }
}

char *splice_app_user_home(void) {
    struct passwd user;
    struct passwd *result;
    char buffer[16384];
    if (getpwuid_r(geteuid(), &user, buffer, sizeof(buffer), &result) != 0 ||
        result == NULL || user.pw_dir == NULL || user.pw_dir[0] != '/') {
        return NULL;
    }
    return strdup(user.pw_dir);
}
