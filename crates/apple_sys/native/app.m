#include "platform.h"

#import <AppKit/AppKit.h>

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
