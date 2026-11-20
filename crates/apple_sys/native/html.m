// HTML escape hatch (PLAN.md T45.1, D17): renders a page frame by frame into a ProRes 4444 alpha
// movie. The page is driven by a virtual clock injected at document start: Date.now,
// performance.now and requestAnimationFrame follow the seek time, every Web Animation (CSS
// animations and transitions included) is paused and set, and a page may define
// `window.seek(seconds)` (the HyperFrames contract) for its own timeline. The same frame must
// come out identical when rendered again out of order, or the page is refused.
#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <WebKit/WebKit.h>
#include "platform.h"

static NSString *const kClock =
    @"(() => {"
     "  let now = 0; const origin = 1767225600000; const queue = [];"
     "  Date.now = () => origin + now * 1000;"
     "  performance.now = () => now * 1000;"
     "  window.requestAnimationFrame = (f) => { queue.push(f); return queue.length; };"
     "  window.cancelAnimationFrame = () => {};"
     "  window.__spliceSeek = (seconds) => {"
     "    now = seconds;"
     "    if (typeof window.seek === 'function') window.seek(seconds);"
     "    for (const a of document.getAnimations()) { a.pause(); a.currentTime = seconds * 1000; }"
     "    const due = queue.splice(0); for (const f of due) f(seconds * 1000);"
     "    return document.readyState;"
     "  };"
     "})();";

@interface SpliceHTMLLoad : NSObject <WKNavigationDelegate>
@property(nonatomic) BOOL done;
@property(nonatomic, strong) NSError *failure;
@end

@implementation SpliceHTMLLoad
- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    self.done = YES;
}
- (void)webView:(WKWebView *)webView didFailNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    self.failure = error;
    self.done = YES;
}
- (void)webView:(WKWebView *)webView didFailProvisionalNavigation:(WKNavigation *)navigation
      withError:(NSError *)error {
    self.failure = error;
    self.done = YES;
}
@end

static bool spin(BOOL (^ready)(void), double seconds) {
    NSDate *limit = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while (!ready()) {
        if ([limit timeIntervalSinceNow] < 0) return false;
        [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                 beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.005]];
    }
    return true;
}

// Seeks the page and captures it as straight BGRA pixels in `pixel` (w × h).
static bool frame(WKWebView *web, double seconds, CVPixelBufferRef pixel, char **error) {
    __block BOOL seeked = NO;
    __block NSError *script = nil;
    [web evaluateJavaScript:[NSString stringWithFormat:@"window.__spliceSeek(%.6f)", seconds]
          completionHandler:^(id result, NSError *failure) {
              (void)result;
              script = failure;
              seeked = YES;
          }];
    if (!spin(^{ return seeked; }, 10) || script != nil) {
        splice_set_error(error, script != nil ? script.localizedDescription.UTF8String
                                              : "The page did not answer the seek.");
        return false;
    }
    size_t w = CVPixelBufferGetWidth(pixel), h = CVPixelBufferGetHeight(pixel);
    WKSnapshotConfiguration *config = [WKSnapshotConfiguration new];
    config.rect = NSMakeRect(0, 0, web.bounds.size.width, web.bounds.size.height);
    config.snapshotWidth = @(web.bounds.size.width);
    config.afterScreenUpdates = YES;
    __block NSImage *image = nil;
    __block BOOL captured = NO;
    [web takeSnapshotWithConfiguration:config
                     completionHandler:^(NSImage *snapshot, NSError *failure) {
                         (void)failure;
                         image = snapshot;
                         captured = YES;
                     }];
    if (!spin(^{ return captured; }, 10) || image == nil) {
        splice_set_error(error, "WebKit could not capture the page.");
        return false;
    }
    CGImageRef cg = [image CGImageForProposedRect:NULL context:nil hints:nil];
    if (cg == NULL) {
        splice_set_error(error, "WebKit returned an empty capture.");
        return false;
    }
    CVPixelBufferLockBaseAddress(pixel, 0);
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(CVPixelBufferGetBaseAddress(pixel), w, h, 8,
                                                 CVPixelBufferGetBytesPerRow(pixel), space,
                                                 kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst);
    CGColorSpaceRelease(space);
    if (context != NULL) {
        CGContextClearRect(context, CGRectMake(0, 0, w, h));
        CGContextDrawImage(context, CGRectMake(0, 0, w, h), cg);
        CGContextRelease(context);
    }
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    if (context == NULL) {
        splice_set_error(error, "Could not draw the captured page.");
        return false;
    }
    return true;
}

static uint64_t digest(CVPixelBufferRef pixel) {
    CVPixelBufferLockBaseAddress(pixel, kCVPixelBufferLock_ReadOnly);
    const uint8_t *bytes = CVPixelBufferGetBaseAddress(pixel);
    size_t length = CVPixelBufferGetBytesPerRow(pixel) * CVPixelBufferGetHeight(pixel);
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < length; i++) hash = (hash ^ bytes[i]) * 0x100000001b3ULL;
    CVPixelBufferUnlockBaseAddress(pixel, kCVPixelBufferLock_ReadOnly);
    return hash;
}

bool splice_html_render(const char *html, const char *base, uint32_t width, uint32_t height, uint32_t fps,
                        uint32_t frames, const char *out, char **error) {
    @autoreleasepool {
        if (html == NULL || out == NULL || width == 0 || height == 0 || width > 8192 || height > 8192 ||
            fps == 0 || frames == 0) {
            splice_set_error(error, "Invalid HTML render request.");
            return false;
        }
        if (![NSThread isMainThread]) {
            splice_set_error(error, "WebKit renders pages on the main thread.");
            return false;
        }
        [NSApplication sharedApplication];
        WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
        [configuration.userContentController
            addUserScript:[[WKUserScript alloc] initWithSource:kClock
                                                 injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                              forMainFrameOnly:YES]];
        NSRect bounds = NSMakeRect(0, 0, width, height);
        WKWebView *web = [[WKWebView alloc] initWithFrame:bounds configuration:configuration];
        [web setValue:@NO forKey:@"drawsBackground"];  // transparent where the page is
        // WebKit paints views in a window: an offscreen, never-shown one.
        NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(-20000, -20000, width, height)
                                                       styleMask:NSWindowStyleMaskBorderless
                                                         backing:NSBackingStoreBuffered
                                                           defer:NO];
        window.contentView = web;
        [window orderBack:nil];
        SpliceHTMLLoad *load = [SpliceHTMLLoad new];
        web.navigationDelegate = load;
        NSURL *root = base != NULL ? [NSURL fileURLWithPath:@(base) isDirectory:YES] : nil;
        [web loadHTMLString:@(html) baseURL:root];
        if (!spin(^{ return load.done; }, 30) || load.failure != nil) {
            splice_set_error(error, load.failure != nil ? load.failure.localizedDescription.UTF8String
                                                       : "The page did not finish loading in 30 s.");
            return false;
        }
        // Loaded is not painted: under load WebKit reported the navigation finished before the
        // first paint and every snapshot came back black (identically, so the determinism check
        // passed). Wait for the fonts, then force one committed paint with a discarded snapshot
        // (requestAnimationFrame is throttled in this offscreen window, so it cannot be the signal).
        __block BOOL painted = NO;
        [web callAsyncJavaScript:@"await document.fonts.ready;"
                       arguments:@{}
                         inFrame:nil
                  inContentWorld:WKContentWorld.pageWorld
               completionHandler:^(id result, NSError *failure) {
                   (void)result;
                   (void)failure;
                   painted = YES;
               }];
        if (!spin(^{ return painted; }, 10)) {
            splice_set_error(error, "The page's fonts did not load within 10 s.");
            return false;
        }
        NSError *failure = nil;
        NSURL *url = [NSURL fileURLWithPath:@(out)];
        [[NSFileManager defaultManager] removeItemAtURL:url error:nil];
        AVAssetWriter *writer = [[AVAssetWriter alloc] initWithURL:url fileType:AVFileTypeQuickTimeMovie
                                                             error:&failure];
        AVAssetWriterInput *input = [AVAssetWriterInput
            assetWriterInputWithMediaType:AVMediaTypeVideo
                           outputSettings:@{
                               AVVideoCodecKey: AVVideoCodecTypeAppleProRes4444,
                               AVVideoColorPropertiesKey: splice_video_color(),
                               AVVideoWidthKey: @(width),
                               AVVideoHeightKey: @(height),
                           }];
        if (writer == nil || ![writer canAddInput:input]) {
            splice_set_error(error, "Could not open the HTML movie for writing.");
            return false;
        }
        input.expectsMediaDataInRealTime = NO;
        [writer addInput:input];
        AVAssetWriterInputPixelBufferAdaptor *adaptor =
            [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input
                                                                            sourcePixelBufferAttributes:nil];
        if (![writer startWriting]) {
            splice_set_error(error, "Could not start the HTML movie.");
            return false;
        }
        [writer startSessionAtSourceTime:kCMTimeZero];
        CVPixelBufferRef pixel = NULL;
        NSDictionary *attributes = @{(id)kCVPixelBufferCGImageCompatibilityKey: @YES,
                                     (id)kCVPixelBufferCGBitmapContextCompatibilityKey: @YES};
        if (CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                                (__bridge CFDictionaryRef)attributes, &pixel) != kCVReturnSuccess) {
            splice_set_error(error, "Could not allocate the HTML frame.");
            return false;
        }
        // The discarded warm-up: a snapshot after screen updates commits the first paint.
        if (!frame(web, 0, pixel, error)) {
            CFRelease(pixel);
            [writer cancelWriting];
            return false;
        }
        // The determinism gate: frames 0 and the middle are rendered again after the pass.
        uint32_t middle = frames / 2;
        uint64_t first = 0, center = 0;
        bool ok = true;
        for (uint32_t index = 0; ok && index < frames; index++) {
            ok = frame(web, (double)index / fps, pixel, error);
            if (!ok) break;
            if (index == 0) first = digest(pixel);
            if (index == middle) center = digest(pixel);
            ok = spin(^{ return input.readyForMoreMediaData; }, 10) &&
                 [adaptor appendPixelBuffer:pixel withPresentationTime:CMTimeMake(index, (int32_t)fps)];
            if (!ok) splice_set_error(error, "Could not encode an HTML frame.");
        }
        if (ok) {
            ok = frame(web, (double)middle / fps, pixel, error) && digest(pixel) == center &&
                 frame(web, 0, pixel, error) && digest(pixel) == first;
            if (!ok && (error == NULL || *error == NULL)) {
                splice_set_error(error,
                                 "The page renders differently when a frame is drawn again: it is not "
                                 "deterministic. Drive its motion from window.seek(seconds), CSS animations or "
                                 "requestAnimationFrame time, not from timers or randomness.");
            }
        }
        CFRelease(pixel);
        if (!ok) {
            [writer cancelWriting];
            return false;
        }
        [input markAsFinished];
        [writer endSessionAtSourceTime:CMTimeMake(frames, (int32_t)fps)];
        __block BOOL finished = NO;
        [writer finishWritingWithCompletionHandler:^{ finished = YES; }];
        if (!spin(^{ return finished; }, 30) || writer.status != AVAssetWriterStatusCompleted) {
            splice_set_error(error, "Could not finish the HTML movie.");
            return false;
        }
        return true;
    }
}
