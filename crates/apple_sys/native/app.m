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

// The capture agent shares the bundle with the viewer, so Launch Services hands it documents
// and reopen events while it runs. It never shows them itself: it starts a viewer (T10.2).
static void open_viewer(NSArray<NSString *> *arguments) {
    NSWorkspaceOpenConfiguration *configuration = [NSWorkspaceOpenConfiguration configuration];
    configuration.createsNewApplicationInstance = YES;
    configuration.arguments = arguments;
    [[NSWorkspace sharedWorkspace] openApplicationAtURL:NSBundle.mainBundle.bundleURL
                                          configuration:configuration
                                      completionHandler:nil];
}

@implementation SpliceAccessoryDelegate
- (void)application:(NSApplication *)sender openURLs:(NSArray<NSURL *> *)urls {
    (void)sender;
    for (NSURL *url in urls) {
        if (url.isFileURL) open_viewer(@[url.path]);
    }
}

- (BOOL)applicationShouldHandleReopen:(NSApplication *)sender hasVisibleWindows:(BOOL)visible {
    (void)sender;
    (void)visible;
    open_viewer(@[]);
    return NO;
}

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

void splice_app_alert(const char *title, const char *message) {
    if (![NSThread isMainThread]) return;
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        [app activateIgnoringOtherApps:YES];
        NSAlert *alert = [NSAlert new];
        alert.alertStyle = NSAlertStyleWarning;
        alert.messageText = [NSString stringWithUTF8String:title] ?: @"Splice";
        alert.informativeText = [NSString stringWithUTF8String:message] ?: @"";
        [alert runModal];
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
