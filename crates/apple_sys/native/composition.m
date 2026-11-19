#include "platform.h"

#import <Accelerate/Accelerate.h>
#import <AVFoundation/AVFoundation.h>
#import <CoreImage/CoreImage.h>
#import <CoreText/CoreText.h>
#import <CoreVideo/CoreVideo.h>
#import <ImageIO/ImageIO.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>

#include <dispatch/dispatch.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

enum { SourceVideo, SourceAudio, SourceStill, SourceColor };

@interface SpliceAVTrack : NSObject
@property(nonatomic, strong) AVMutableCompositionTrack *track;
// A solid or gradient fill: the compositor draws it instead of decoding its still movie.
@property(nonatomic) BOOL fill;
@property(nonatomic) uint32_t colorA, colorB;
@property(nonatomic, strong) AVMutableAudioMixInputParameters *mix;
@property(nonatomic) CGSize size;
@property(nonatomic) CGAffineTransform orientation;
@end
@implementation SpliceAVTrack
@end

@interface SpliceAVTextLayer : CALayer
@property(nonatomic, strong) CIImage *image;
@property(nonatomic) int32_t firstFrame;
@property(nonatomic) int32_t endFrame;
@property(nonatomic) float fromOpacity;
@property(nonatomic) float toOpacity;
@end

@implementation SpliceAVTextLayer
@end

@interface SpliceAVAssembly : NSObject
@property(nonatomic) CGSize size;
@property(nonatomic) int32_t fps;
@property(nonatomic) int32_t end;
@property(nonatomic) int32_t covered;
@property(nonatomic, strong) AVMutableComposition *asset;
@property(nonatomic, strong) NSMutableArray<SpliceAVTrack *> *tracks;
@property(nonatomic, strong) NSMutableArray *instructions;
@property(nonatomic, strong) NSMutableArray *cursors;
@property(nonatomic, strong) AVMutableVideoComposition *brokered;
// Source buffers per frame from brokered requests, newest last (at most 8). Guarded by @synchronized.
@property(nonatomic, strong) NSMutableArray<NSArray *> *frames;
@property(nonatomic, strong) NSMutableDictionary<NSURL *, AVURLAsset *> *inputs;
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSURL *> *stills;
@property(nonatomic, strong) NSURL *scratch;
@property(nonatomic, strong) CALayer *text;
@property(nonatomic, strong) AVMutableVideoComposition *video;
@property(nonatomic, strong) AVMutableVideoComposition *offline;
@property(nonatomic, strong) AVMutableAudioMix *audio;
@property(nonatomic, strong) AVAssetImageGenerator *generator;
@end

@implementation SpliceAVAssembly
- (void)dealloc {
    if (_scratch != nil) {
        [[NSFileManager defaultManager] removeItemAtURL:_scratch error:NULL];
    }
}
@end

@interface SpliceAVExport : NSObject
@property(nonatomic, strong) SpliceAVAssembly *assembly;
@property(nonatomic, strong) AVAssetExportSession *session;
@end

@interface SpliceAVPlayer : NSObject
@property(nonatomic, strong) SpliceAVAssembly *assembly;
@property(nonatomic, strong) NSMutableDictionary<NSNumber *, id<MTLTexture>> *surfaces;
@property(nonatomic, strong) NSMutableArray *slots;  // three NSArrays of retained source buffers
@property(nonatomic, strong) AVPlayer *player;
@property(nonatomic, strong) AVPlayerItemVideoOutput *output;
@end

@implementation SpliceAVPlayer
- (void)dealloc {
    [_player pause];
    [_player replaceCurrentItemWithPlayerItem:nil];
}
@end



// These Objective-C editing APIs support the deployment target, including macOS 15.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static void fail(char **error, NSError *native, const char *fallback) {
    const char *description = native.localizedDescription.UTF8String;
    splice_set_error(error, description != NULL ? description : fallback);
}

static CGRect rect(const double values[4]) {
    return CGRectMake(values[0], values[1], values[2], values[3]);
}

static bool valid_rect(const double values[4]) {
    return isfinite(values[0]) && isfinite(values[1]) &&
           isfinite(values[2]) && isfinite(values[3]) && values[2] > 0 && values[3] > 0;
}

static bool positive(CMTime time) {
    return CMTIME_IS_NUMERIC(time) && CMTimeCompare(time, kCMTimeZero) > 0;
}

static CMTime time_for(SpliceAVAssembly *assembly, int32_t frame) {
    return CMTimeMake(frame, assembly.fps);
}

static CMTime seconds(double value) {
    return CMTimeMakeWithSeconds(value, 600000000);
}

static void rgb(uint32_t value, CGFloat out[4]) {
    out[0] = ((value >> 16) & 255) / 255.0;
    out[1] = ((value >> 8) & 255) / 255.0;
    out[2] = (value & 255) / 255.0;
    out[3] = 1.0;
}

static CGImageRef still_image(NSURL *file) {
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)file, NULL);
    if (source == NULL) return NULL;
    NSDictionary *options = @{
        (__bridge NSString *)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
        (__bridge NSString *)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (__bridge NSString *)kCGImageSourceThumbnailMaxPixelSize: @8192,
    };
    CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    return image;
}

static CVPixelBufferRef still_buffer(CGImageRef image, const uint32_t colors[2]) {
    // The smallest encoded frame is kept even-sized for hardware encoders.
    size_t width = image != NULL ? MAX(32, (CGImageGetWidth(image) + 1) & ~(size_t)1) : 32;
    size_t height = image != NULL ? MAX(32, (CGImageGetHeight(image) + 1) & ~(size_t)1) : 32;
    CVPixelBufferRef pixel = NULL;
    if (CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA,
                            (__bridge CFDictionaryRef)@{(id)kCVPixelBufferCGImageCompatibilityKey: @YES},
                            &pixel) != kCVReturnSuccess) return NULL;
    if (CVPixelBufferLockBaseAddress(pixel, 0) != kCVReturnSuccess) {
        CFRelease(pixel);
        return NULL;
    }
    CGColorSpaceRef color_space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(CVPixelBufferGetBaseAddress(pixel), width, height, 8,
                                                 CVPixelBufferGetBytesPerRow(pixel), color_space,
                                                 kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst);
    CGRect bounds = CGRectMake(0, 0, width, height);
    if (context != NULL) {
        CGContextClearRect(context, bounds);
        if (image != NULL) {
            CGContextDrawImage(context, bounds, image);
        } else {
            CGFloat values[8];
            rgb(colors[0], values);
            rgb(colors[1], values + 4);
            CGGradientRef gradient = CGGradientCreateWithColorComponents(color_space, values, NULL, 2);
            if (gradient != NULL) {
                CGContextDrawLinearGradient(context, gradient, CGPointMake(0, height), CGPointMake(width, 0), 0);
                CGGradientRelease(gradient);
            }
        }
        CGContextRelease(context);
    }
    CGColorSpaceRelease(color_space);
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    if (context == NULL) {
        CFRelease(pixel);
        return NULL;
    }
    return pixel;
}

static bool write_still(NSURL *url, CVPixelBufferRef pixel, char **error) {
    NSError *failure = nil;
    AVAssetWriter *writer = [[AVAssetWriter alloc] initWithURL:url fileType:AVFileTypeQuickTimeMovie error:&failure];
    if (writer == nil) {
        fail(error, failure, "Could not open a temporary picture track.");
        return false;
    }
    AVAssetWriterInput *input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:@{
        AVVideoCodecKey: AVVideoCodecTypeAppleProRes4444,
        AVVideoWidthKey: @(CVPixelBufferGetWidth(pixel)),
        AVVideoHeightKey: @(CVPixelBufferGetHeight(pixel)),
    }];
    if (![writer canAddInput:input]) {
        fail(error, writer.error, "Could not encode a picture track.");
        return false;
    }
    [writer addInput:input];
    AVAssetWriterInputPixelBufferAdaptor *adaptor =
        [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input
                                                                        sourcePixelBufferAttributes:nil];
    if (![writer startWriting]) {
        fail(error, writer.error, "Could not start the picture encoder.");
        return false;
    }
    [writer startSessionAtSourceTime:kCMTimeZero];
    dispatch_semaphore_t complete = dispatch_semaphore_create(0);
    id pixel_owner = (__bridge id)pixel;
    __block bool appended = false;
    [input requestMediaDataWhenReadyOnQueue:dispatch_queue_create("splice.still", DISPATCH_QUEUE_SERIAL) usingBlock:^{
        if (appended) return;
        appended = true;
        if (writer.status != AVAssetWriterStatusWriting) {
            dispatch_semaphore_signal(complete);
            return;
        }
        if (![adaptor appendPixelBuffer:(__bridge CVPixelBufferRef)pixel_owner withPresentationTime:kCMTimeZero]) {
            [writer cancelWriting];
            dispatch_semaphore_signal(complete);
            return;
        }
        [writer endSessionAtSourceTime:CMTimeMake(1, 30)];
        [input markAsFinished];
        [writer finishWritingWithCompletionHandler:^{ dispatch_semaphore_signal(complete); }];
    }];
    if (dispatch_semaphore_wait(complete, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
        [writer cancelWriting];
        splice_set_error(error, "Timed out encoding a temporary picture track.");
        return false;
    }
    if (writer.status != AVAssetWriterStatusCompleted) {
        fail(error, writer.error, "Could not encode the temporary picture track.");
        return false;
    }
    return true;
}

static NSURL *still_movie(SpliceAVAssembly *assembly, NSURL *file, const uint32_t colors[2], char **error) {
    NSString *key = file != nil ? file.absoluteString : [NSString stringWithFormat:@"rgb:%u:%u", colors[0], colors[1]];
    NSURL *existing = assembly.stills[key];
    if (existing != nil) return existing;
    if (assembly.scratch == nil) {
        NSString *temp = [NSTemporaryDirectory() stringByAppendingPathComponent:@"splice-av.XXXXXX"];
        const char *representation = temp.fileSystemRepresentation;
        char *path = representation != NULL ? strdup(representation) : NULL;
        if (path != NULL && mkdtemp(path) != NULL) {
            assembly.scratch = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:YES relativeToURL:nil];
        }
        free(path);
        if (assembly.scratch == nil) {
            splice_set_error(error, "Could not create a private directory for picture tracks.");
            return nil;
        }
    }
    CGImageRef image = file != nil ? still_image(file) : NULL;
    if (file != nil && image == NULL) {
        fail(error, nil, "ImageIO could not read a still image.");
        return nil;
    }
    CVPixelBufferRef pixel = still_buffer(image, colors);
    if (image != NULL) CGImageRelease(image);
    if (pixel == NULL) {
        splice_set_error(error, "Could not allocate the still image buffer.");
        return nil;
    }
    NSURL *url = [assembly.scratch URLByAppendingPathComponent:[NSString stringWithFormat:@"%lu.mov", assembly.stills.count]];
    bool written = write_still(url, pixel, error);
    CFRelease(pixel);
    if (!written) return nil;
    assembly.stills[key] = url;
    return url;
}

static bool insert(AVMutableCompositionTrack *destination, AVAssetTrack *input, CMTimeRange source,
                   CMTime at, CMTime duration, char **error) {
    NSError *failure = nil;
    if (![destination insertTimeRange:source ofTrack:input atTime:at error:&failure]) {
        fail(error, failure, "Could not add a source segment to the composition.");
        return false;
    }
    if (CMTimeCompare(source.duration, duration) != 0) {
        [destination scaleTimeRange:CMTimeRangeMake(at, source.duration) toDuration:duration];
    }
    return true;
}

static bool place(SpliceAVAssembly *assembly, AVMutableCompositionTrack *destination, AVAssetTrack *input,
                  const SpliceAVSource *spec, char **error) {
    CMTimeRange available = input.timeRange;
    CMTime start = time_for(assembly, spec->timeline[0]);
    CMTime end = time_for(assembly, spec->timeline[1]);
    if (spec->kind >= SourceStill) {
        return insert(destination, input, available, start, CMTimeSubtract(end, start), error);
    }
    CMTime wanted = seconds(spec->source_start);
    CMTime limit = CMTimeMinimum(CMTimeRangeGetEnd(available), seconds(spec->source_limit));
    CMTime first = CMTimeMaximum(wanted, available.start);
    CMTime last = CMTimeMinimum(limit, CMTimeAdd(wanted, seconds(CMTimeGetSeconds(CMTimeSubtract(end, start)) * spec->speed)));
    CMTime timeline = CMTimeAdd(start, seconds((CMTimeGetSeconds(first) - spec->source_start) / spec->speed));
    if (CMTimeCompare(last, first) > 0 && CMTimeCompare(timeline, end) < 0) {
        CMTime length = CMTimeMinimum(CMTimeSubtract(end, timeline), seconds(CMTimeGetSeconds(CMTimeSubtract(last, first)) / spec->speed));
        if (!insert(destination, input, CMTimeRangeFromTimeToTime(first, last), timeline, length, error)) return false;
        timeline = CMTimeAdd(timeline, length);
    } else {
        timeline = start;
    }
    if (spec->kind == SourceVideo && CMTimeCompare(timeline, end) < 0) {
        CMTime hold = CMTimeMaximum(available.start, CMTimeSubtract(limit, seconds(spec->source_frame_duration)));
        if (CMTimeCompare(limit, hold) <= 0) {
            splice_set_error(error, "The source video has no frame in the requested interval.");
            return false;
        }
        return insert(destination, input, CMTimeRangeFromTimeToTime(hold, limit),
                      timeline, CMTimeSubtract(end, timeline), error);
    }
    return true;
}

void *splice_av_create(uint32_t width, uint32_t height, uint32_t fps, int32_t end, char **error) {
    @autoreleasepool {
        if (width == 0 || height == 0 || width > 16384 || height > 16384 || fps == 0 ||
            fps > INT32_MAX || end <= 0) {
            splice_set_error(error, "The composition requires a supported canvas, frame rate, and nonempty timeline.");
            return NULL;
        }
        SpliceAVAssembly *assembly = [SpliceAVAssembly new];
        assembly.size = CGSizeMake(width, height);
        assembly.fps = (int32_t)fps;
        assembly.end = end;
        assembly.asset = [AVMutableComposition composition];
        assembly.asset.naturalSize = assembly.size;
        assembly.tracks = [NSMutableArray array];
        assembly.instructions = [NSMutableArray array];
        assembly.inputs = [NSMutableDictionary dictionary];
        assembly.stills = [NSMutableDictionary dictionary];
        return (__bridge_retained void *)assembly;
    }
}

void splice_av_release(void *raw) {
    @autoreleasepool {
        (void)CFBridgingRelease(raw);
    }
}

int32_t splice_av_source(void *raw, const SpliceAVSource *spec, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        if (spec->timeline[0] < 0 || spec->timeline[0] >= spec->timeline[1] ||
            spec->timeline[1] > assembly.end || spec->kind > SourceColor ||
            !isfinite(spec->source_start) || !isfinite(spec->source_limit) ||
            !isfinite(spec->speed) || spec->speed <= 0 || spec->source_start < 0 ||
            !isfinite(spec->source_frame_duration) || spec->source_frame_duration <= 0 ||
            (spec->kind < SourceStill && spec->source_limit <= spec->source_start)) {
            splice_set_error(error, "The input has an invalid composition time range.");
            return -1;
        }
        NSURL *url = spec->path == NULL ? nil : [NSURL fileURLWithFileSystemRepresentation:spec->path
                                                                            isDirectory:NO relativeToURL:nil];
        if (spec->kind != SourceColor && url == nil) {
            splice_set_error(error, "The composition source URL is missing.");
            return -1;
        }
        if (spec->kind >= SourceStill) {
            url = still_movie(assembly, url, spec->colors, error);
            if (url == nil) return -1;
        }
        AVURLAsset *asset = assembly.inputs[url];
        if (asset == nil) {
            asset = [AVURLAsset URLAssetWithURL:url options:@{AVURLAssetPreferPreciseDurationAndTimingKey: @YES}];
            assembly.inputs[url] = asset;
        }
        bool audio = spec->kind == SourceAudio;
        bool fill = spec->kind == SourceColor;
        AVMediaType type = audio ? AVMediaTypeAudio : AVMediaTypeVideo;
        NSArray<AVAssetTrack *> *inputs = [asset tracksWithMediaType:type];
        if (spec->index >= inputs.count || assembly.tracks.count >= INT32_MAX) {
            splice_set_error(error, [[NSString stringWithFormat:@"Could not find the requested %@ track in %@.", type, url.path] UTF8String]);
            return -1;
        }
        AVAssetTrack *input = inputs[spec->index];
        if (!positive(input.timeRange.duration)) {
            splice_set_error(error, "The source track has no usable duration.");
            return -1;
        }
        AVMutableCompositionTrack *track = [assembly.asset addMutableTrackWithMediaType:type
                                                                       preferredTrackID:kCMPersistentTrackID_Invalid];
        if (track == nil) {
            splice_set_error(error, "AVFoundation could not allocate a composition track.");
            return -1;
        }
        if (!place(assembly, track, input, spec, error)) return -1;
        SpliceAVTrack *entry = [SpliceAVTrack new];
        entry.track = track;
        entry.fill = fill;
        entry.colorA = spec->colors[0];
        entry.colorB = spec->colors[1];
        if (audio) {
            entry.mix = [AVMutableAudioMixInputParameters audioMixInputParametersWithTrack:track];
        } else {
            entry.size = input.naturalSize;
            entry.orientation = input.preferredTransform;
            if (entry.size.width <= 0 || entry.size.height <= 0) {
                splice_set_error(error, "The source video has invalid geometry.");
                return -1;
            }
        }
        int32_t index = (int32_t)assembly.tracks.count;
        [assembly.tracks addObject:entry];
        return index;
    }
}

static SpliceAVTrack *track_at(SpliceAVAssembly *assembly, int32_t index) {
    return index >= 0 && (NSUInteger)index < assembly.tracks.count ? assembly.tracks[index] : nil;
}

// ── Compositor (PLAN.md D1): Core Image on Metal, driven by per-interval layer ramps ──
// ponytail: interim macOS renderer. The plan (ramps, zoom keyframes, cursor tables) is portable Rust;
// the renderer moves to OGPU (shared device and textures with PXUI, IOSurface/dmabuf import) in T12.7.
// Upgrade trigger: T12.7, or the 4K viewer budget (8 ms) in window_program_4k.

@interface SpliceCursorTable : NSObject
@property(nonatomic, strong) NSData *samples;
@property(nonatomic, strong) NSArray<CIImage *> *images;
@property(nonatomic, strong) NSData *hotspots;  // double[3] per image: hot x, hot y, pixels per point
@property(nonatomic) double points;
@end
@implementation SpliceCursorTable
@end

@interface SpliceInstruction : NSObject <AVVideoCompositionInstruction>
@property(nonatomic) CMTimeRange timeRange;
@property(nonatomic) BOOL enablePostProcessing;
@property(nonatomic) BOOL containsTweening;
@property(nonatomic, strong, nullable) NSArray<NSValue *> *requiredSourceTrackIDs;
@property(nonatomic) CMPersistentTrackID passthroughTrackID;
@property(nonatomic, strong) NSData *layers;
@property(nonatomic, strong) NSArray<SpliceAVTrack *> *sources;  // one per layer
@property(nonatomic, strong) NSArray<SpliceCursorTable *> *cursors;
// Titles, captions and keystroke chips, drawn only by the offline (export) composition: the viewer
// draws them in PXUI and frame grabs overlay them in splice_av_read_frame.
@property(nonatomic, strong, nullable) NSArray<SpliceAVTextLayer *> *texts;
// The player's instructions only collect source frames for the OGPU compositor (T12.7).
@property(nonatomic, weak) SpliceAVAssembly *broker;
@property(nonatomic) CGSize canvas;
@property(nonatomic) int32_t fps;
@end
@implementation SpliceInstruction
@end

static double lerp(double a, double b, double t) { return a + (b - a) * t; }

// Converts a top-left canvas rectangle to Core Image's bottom-left coordinates.
static CGRect flip(CGRect r, double height) {
    return CGRectMake(r.origin.x, height - r.origin.y - r.size.height, r.size.width, r.size.height);
}

static const SpliceAVCursorSample *cursor_at(SpliceCursorTable *table, double t, double out[2]) {
    const SpliceAVCursorSample *samples = table.samples.bytes;
    size_t count = table.samples.length / sizeof(SpliceAVCursorSample), lo = 0, hi = count;
    while (lo < hi) {
        size_t mid = (lo + hi) / 2;
        if (samples[mid].t <= t) lo = mid + 1; else hi = mid;
    }
    if (lo == 0) return NULL;
    const SpliceAVCursorSample *a = &samples[lo - 1];
    out[0] = a->x, out[1] = a->y;
    // Interpolate across short gaps between position samples, so the cursor glides.
    if (lo < count && samples[lo].t - a->t < 0.25 && samples[lo].t > a->t) {
        double k = (t - a->t) / (samples[lo].t - a->t);
        out[0] = lerp(a->x, samples[lo].x, k), out[1] = lerp(a->y, samples[lo].y, k);
    }
    return a;
}

@interface SpliceCompositor : NSObject <AVVideoCompositing>
@end

@implementation SpliceCompositor {
    CIContext *_context;
    CGColorSpaceRef _space;
}

- (instancetype)init {
    if ((self = [super init])) {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        NSDictionary *options = @{kCIContextWorkingColorSpace: [NSNull null], kCIContextOutputColorSpace: [NSNull null],
                                  kCIContextCacheIntermediates: @NO};
        _context = device != nil ? [CIContext contextWithMTLDevice:device options:options]
                                 : [CIContext contextWithOptions:options];
    }
    return self;
}

- (NSDictionary *)sourcePixelBufferAttributes {
    return @{(id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
             (id)kCVPixelBufferMetalCompatibilityKey: @YES};
}

- (NSDictionary *)requiredPixelBufferAttributesForRenderContext {
    // CPU-memory output: Core Image reads back on the compositor thread, so the viewer's copy on
    // the UI thread reads cached memory (a GPU-written IOSurface tripled its 4K copy time).
    return @{(id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA)};
}

- (void)renderContextChanged:(AVVideoCompositionRenderContext *)context {
    (void)context;
}

- (void)cancelAllPendingVideoCompositionRequests {
}

- (void)startVideoCompositionRequest:(AVAsynchronousVideoCompositionRequest *)request {
    @autoreleasepool {
        SpliceInstruction *instruction = (SpliceInstruction *)request.videoCompositionInstruction;
        CVPixelBufferRef output = [request.renderContext newPixelBuffer];
        if (output == NULL) {
            [request finishWithError:[NSError errorWithDomain:@"Splice" code:1 userInfo:nil]];
            return;
        }
        SpliceAVAssembly *broker = instruction.broker;
        if (broker != nil) {
            // Keep the sources for the viewer; the output buffer only carries the timing.
            size_t count = instruction.layers.length / sizeof(SpliceAVLayer);
            NSMutableArray *entry = [NSMutableArray arrayWithCapacity:count + 1];
            [entry addObject:@((int32_t)llround(CMTimeGetSeconds(request.compositionTime) * instruction.fps))];
            for (size_t index = 0; index < count; index++) {
                SpliceAVTrack *source = instruction.sources[index];
                CVPixelBufferRef pixels = source.fill ? NULL : [request sourceFrameByTrackID:source.track.trackID];
                [entry addObject:pixels != NULL ? (__bridge id)pixels : (id)[NSNull null]];
            }
            @synchronized (broker) {
                if (broker.frames == nil) broker.frames = [NSMutableArray array];
                [broker.frames addObject:entry];
                while (broker.frames.count > 8) [broker.frames removeObjectAtIndex:0];
            }
            [request finishWithComposedVideoFrame:output];
            CVPixelBufferRelease(output);
            return;
        }
        CGSize canvas = instruction.canvas;
        CGRect bounds = CGRectMake(0, 0, canvas.width, canvas.height);
        double span = CMTimeGetSeconds(instruction.timeRange.duration);
        double linear = span > 0 ? CMTimeGetSeconds(CMTimeSubtract(request.compositionTime,
                                                                   instruction.timeRange.start)) / span : 0;
        linear = fmin(fmax(linear, 0), 1);
        CIImage *frame = [[CIImage imageWithColor:[CIColor blackColor]] imageByCroppingToRect:bounds];
        const SpliceAVLayer *layers = instruction.layers.bytes;
        size_t count = instruction.layers.length / sizeof(SpliceAVLayer);
        for (size_t index = 0; index < count; index++) {
            const SpliceAVLayer *layer = &layers[index];
            SpliceAVTrack *source = instruction.sources[index];
            double t = (layer->flags & SpliceAVEase) ? linear * linear * (3 - 2 * linear) : linear;
            CIImage *image;
            if (source.fill) {
                // Same diagonal as still_buffer: colorA top-left to colorB bottom-right.
                CGFloat a[4], b[4];
                rgb(source.colorA, a), rgb(source.colorB, b);
                CGRect area = flip(rect(layer->destination), canvas.height);
                CIColor *from = [CIColor colorWithRed:a[0] green:a[1] blue:a[2]];
                CIColor *to = [CIColor colorWithRed:b[0] green:b[1] blue:b[2]];
                image = source.colorA == source.colorB ? [CIImage imageWithColor:from]
                    : [CIFilter filterWithName:@"CILinearGradient" withInputParameters:@{
                        @"inputPoint0": [CIVector vectorWithX:CGRectGetMinX(area) Y:CGRectGetMaxY(area)],
                        @"inputPoint1": [CIVector vectorWithX:CGRectGetMaxX(area) Y:CGRectGetMinY(area)],
                        @"inputColor0": from, @"inputColor1": to}].outputImage;
                image = [image imageByCroppingToRect:area];
                double opacity = lerp(layer->opacity[0], layer->opacity[1], linear);
                if (opacity < 1) {
                    image = [image imageByApplyingFilter:@"CIColorMatrix" withInputParameters:@{
                        @"inputRVector": [CIVector vectorWithX:opacity Y:0 Z:0 W:0],
                        @"inputGVector": [CIVector vectorWithX:0 Y:opacity Z:0 W:0],
                        @"inputBVector": [CIVector vectorWithX:0 Y:0 Z:opacity W:0],
                        @"inputAVector": [CIVector vectorWithX:0 Y:0 Z:0 W:opacity]}];
                }
                frame = [image imageByCompositingOverImage:frame];
                continue;
            }
            CVPixelBufferRef pixels = [request sourceFrameByTrackID:source.track.trackID];
            if (pixels == NULL) continue;
            image = [CIImage imageWithCVPixelBuffer:pixels];
            // Orient in top-left space, then return to Core Image's bottom-left space.
            CGSize natural = image.extent.size;
            CGRect oriented = CGRectApplyAffineTransform(CGRectMake(0, 0, natural.width, natural.height), source.orientation);
            CGAffineTransform to_top = CGAffineTransformMake(1, 0, 0, -1, 0, natural.height);
            CGAffineTransform orient = CGAffineTransformConcat(source.orientation,
                CGAffineTransformMakeTranslation(-oriented.origin.x, -oriented.origin.y));
            CGAffineTransform to_bottom = CGAffineTransformMake(1, 0, 0, -1, 0, oriented.size.height);
            image = [image imageByApplyingTransform:CGAffineTransformConcat(CGAffineTransformConcat(to_top, orient), to_bottom)];
            double W = oriented.size.width, H = oriented.size.height;
            CGRect crop = CGRectMake(layer->crop[0] * W, layer->crop[1] * H, layer->crop[2] * W, layer->crop[3] * H);
            CGRect dest = rect(layer->destination);
            if (layer->center_crop != 0) {
                double ratio = dest.size.width / dest.size.height;
                double width = fmin(crop.size.width, crop.size.height * ratio);
                double height = fmin(crop.size.height, crop.size.width / ratio);
                crop = CGRectInset(crop, (crop.size.width - width) / 2, (crop.size.height - height) / 2);
            }
            double scale = lerp(layer->scale[0], layer->scale[1], t);
            double fx = lerp(layer->focus[0], layer->focus[2], t), fy = lerp(layer->focus[1], layer->focus[3], t);
            CGRect placed = CGRectMake(fx + (dest.origin.x - fx) * scale, fy + (dest.origin.y - fy) * scale,
                                       dest.size.width * scale, dest.size.height * scale);
            CGRect crop_ci = flip(crop, H), placed_ci = flip(placed, canvas.height);
            image = [image imageByCroppingToRect:crop_ci];
            CGAffineTransform fit = CGAffineTransformMakeTranslation(-crop_ci.origin.x, -crop_ci.origin.y);
            fit = CGAffineTransformConcat(fit, CGAffineTransformMakeScale(placed.size.width / crop.size.width,
                                                                          placed.size.height / crop.size.height));
            fit = CGAffineTransformConcat(fit, CGAffineTransformMakeTranslation(placed_ci.origin.x, placed_ci.origin.y));
            image = [[image imageByApplyingTransform:fit] imageByCroppingToRect:placed_ci];
            CIImage *mask = nil;
            double corner = layer->corner * scale;
            if (corner > 0.5 || layer->shadow > 0) {
                mask = [CIFilter filterWithName:@"CIRoundedRectangleGenerator" withInputParameters:@{
                    @"inputExtent": [CIVector vectorWithCGRect:placed_ci], @"inputRadius": @(fmax(corner, 0)),
                    @"inputColor": [CIColor whiteColor]}].outputImage;
                if (corner > 0.5) {
                    image = [image imageByApplyingFilter:@"CISourceInCompositing"
                                     withInputParameters:@{kCIInputBackgroundImageKey: mask}];
                }
            }
            // Cursor and click rings ride on the picture, so zoom and crop move them with it.
            SpliceCursorTable *table = layer->cursor >= 0 && (NSUInteger)layer->cursor < instruction.cursors.count
                                           ? instruction.cursors[layer->cursor] : nil;
            if (table != nil) {
                double st = lerp(layer->source[0], layer->source[1], linear), at[2];
                double per_px = placed.size.width / crop.size.width;  // canvas pixels per source pixel
                double px_per_point = W / table.points;
                const SpliceAVCursorSample *samples = table.samples.bytes;
                size_t total = table.samples.length / sizeof(SpliceAVCursorSample);
                CGRect clip = placed_ci;
                if (layer->flags & SpliceAVClickRings) {
                    for (size_t i = 0; i < total; i++) {
                        double age = st - samples[i].t;
                        if (!samples[i].click || age < 0 || age > 0.45) continue;
                        double cx = placed.origin.x + (samples[i].x * W - crop.origin.x) * per_px;
                        double cy = placed.origin.y + (samples[i].y * H - crop.origin.y) * per_px;
                        double radius = (10 + 34 * age / 0.45) * px_per_point * per_px * layer->cursor_scale;
                        CIImage *ring = [CIFilter filterWithName:@"CIRadialGradient" withInputParameters:@{
                            @"inputCenter": [CIVector vectorWithX:cx Y:canvas.height - cy],
                            @"inputRadius0": @(radius * 0.55), @"inputRadius1": @(radius),
                            @"inputColor0": [CIColor colorWithRed:1 green:1 blue:1 alpha:0.6 * (1 - age / 0.45)],
                            @"inputColor1": [CIColor colorWithRed:1 green:1 blue:1 alpha:0]}].outputImage;
                        image = [[ring imageByCroppingToRect:clip] imageByCompositingOverImage:image];
                    }
                }
                const SpliceAVCursorSample *sample = cursor_at(table, st, at);
                if (sample != NULL && sample->image >= 0 && (NSUInteger)sample->image < table.images.count) {
                    CIImage *cursor = table.images[sample->image];
                    const double *hot = &((const double *)table.hotspots.bytes)[sample->image * 3];
                    double k = layer->cursor_scale * px_per_point * per_px / hot[2];  // canvas px per PNG px
                    double x = placed.origin.x + (at[0] * W - crop.origin.x) * per_px - hot[0] * k;
                    double y = placed.origin.y + (at[1] * H - crop.origin.y) * per_px - hot[1] * k;
                    CGSize size = cursor.extent.size;
                    CGAffineTransform move = CGAffineTransformMakeScale(k, k);
                    move = CGAffineTransformConcat(move, CGAffineTransformMakeTranslation(x, canvas.height - y - size.height * k));
                    image = [[[cursor imageByApplyingTransform:move] imageByCroppingToRect:clip] imageByCompositingOverImage:image];
                }
            }
            double opacity = lerp(layer->opacity[0], layer->opacity[1], linear);
            if (layer->shadow > 0 && mask != nil) {
                double blur = 0.03 * fmin(placed.size.width, placed.size.height);
                CIImage *shadow = [mask imageByApplyingFilter:@"CIColorMatrix" withInputParameters:@{
                    @"inputRVector": [CIVector vectorWithX:0 Y:0 Z:0 W:0], @"inputGVector": [CIVector vectorWithX:0 Y:0 Z:0 W:0],
                    @"inputBVector": [CIVector vectorWithX:0 Y:0 Z:0 W:0],
                    @"inputAVector": [CIVector vectorWithX:0 Y:0 Z:0 W:0.55 * layer->shadow]}];
                shadow = [[shadow imageByApplyingGaussianBlurWithSigma:blur / 2]
                    imageByApplyingTransform:CGAffineTransformMakeTranslation(0, -blur / 3)];
                image = [image imageByCompositingOverImage:[shadow imageByCroppingToRect:bounds]];
            }
            if (opacity < 1) {
                image = [image imageByApplyingFilter:@"CIColorMatrix" withInputParameters:@{
                    @"inputRVector": [CIVector vectorWithX:opacity Y:0 Z:0 W:0],
                    @"inputGVector": [CIVector vectorWithX:0 Y:opacity Z:0 W:0],
                    @"inputBVector": [CIVector vectorWithX:0 Y:0 Z:opacity W:0],
                    @"inputAVector": [CIVector vectorWithX:0 Y:0 Z:0 W:opacity]}];
            }
            frame = [image imageByCompositingOverImage:frame];
        }
        int32_t at = (int32_t)llround(CMTimeGetSeconds(request.compositionTime) * instruction.fps);
        for (SpliceAVTextLayer *text in instruction.texts) {
            if (at < text.firstFrame || at >= text.endFrame) continue;
            if (text.image == nil) text.image = [CIImage imageWithCGImage:(__bridge CGImageRef)text.contents];
            double progress = (double)(at - text.firstFrame) / (text.endFrame - text.firstFrame);
            double alpha = text.fromOpacity + (text.toOpacity - text.fromOpacity) * progress;
            CIImage *glyphs = [text.image imageByApplyingTransform:
                CGAffineTransformMakeTranslation(text.frame.origin.x, text.frame.origin.y)];
            if (alpha < 1) {
                glyphs = [glyphs imageByApplyingFilter:@"CIColorMatrix" withInputParameters:@{
                    @"inputRVector": [CIVector vectorWithX:alpha Y:0 Z:0 W:0],
                    @"inputGVector": [CIVector vectorWithX:0 Y:alpha Z:0 W:0],
                    @"inputBVector": [CIVector vectorWithX:0 Y:0 Z:alpha W:0],
                    @"inputAVector": [CIVector vectorWithX:0 Y:0 Z:0 W:alpha]}];
            }
            frame = [glyphs imageByCompositingOverImage:frame];
        }
        [_context render:[frame imageByCroppingToRect:bounds] toCVPixelBuffer:output bounds:bounds colorSpace:nil];
        [request finishWithComposedVideoFrame:output];
        CVPixelBufferRelease(output);
    }
}
@end

int32_t splice_av_cursor(void *raw, const SpliceAVCursorSample *samples, size_t sample_count,
                         const SpliceAVCursorImage *images, size_t image_count, double points, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        if (!(points > 0) || !isfinite(points) || (sample_count > 0 && samples == NULL) ||
            (image_count > 0 && images == NULL)) {
            splice_set_error(error, "Invalid cursor track.");
            return -1;
        }
        NSMutableArray<CIImage *> *loaded = [NSMutableArray arrayWithCapacity:image_count];
        NSMutableData *hotspots = [NSMutableData dataWithLength:image_count * 3 * sizeof(double)];
        double *hot = hotspots.mutableBytes;
        for (size_t index = 0; index < image_count; index++) {
            NSURL *url = [NSURL fileURLWithFileSystemRepresentation:images[index].path isDirectory:NO relativeToURL:nil];
            CIImage *image = url != nil ? [CIImage imageWithContentsOfURL:url] : nil;
            if (image == nil || !(images[index].scale > 0)) {
                splice_set_error(error, "A cursor image could not be loaded.");
                return -1;
            }
            [loaded addObject:image];
            hot[index * 3] = images[index].hot[0], hot[index * 3 + 1] = images[index].hot[1];
            hot[index * 3 + 2] = images[index].scale;
        }
        for (size_t index = 0; index < sample_count; index++) {
            if (!isfinite(samples[index].t) || !isfinite(samples[index].x) || !isfinite(samples[index].y) ||
                samples[index].image >= (int32_t)image_count || (index > 0 && samples[index].t < samples[index - 1].t)) {
                splice_set_error(error, "Cursor samples must be finite, ordered and reference loaded images.");
                return -1;
            }
        }
        SpliceCursorTable *table = [SpliceCursorTable new];
        table.samples = [NSData dataWithBytes:samples length:sample_count * sizeof(SpliceAVCursorSample)];
        table.images = loaded;
        table.hotspots = hotspots;
        table.points = points;
        if (assembly.cursors == nil) assembly.cursors = [NSMutableArray array];
        [assembly.cursors addObject:table];
        return (int32_t)assembly.cursors.count - 1;
    }
}

bool splice_av_range(void *raw, int32_t from, int32_t to,
                     const SpliceAVLayer *layers, size_t layer_count,
                     const SpliceAVGain *gains, size_t gain_count, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        if (from != assembly.covered || to <= from || to > assembly.end || layer_count == 0) {
            splice_set_error(error, "Video instructions must cover the composition in order.");
            return false;
        }
        CMTime start = time_for(assembly, from);
        CMTimeRange interval = CMTimeRangeFromTimeToTime(start, time_for(assembly, to));
        NSMutableArray<SpliceAVTrack *> *sources = [NSMutableArray arrayWithCapacity:layer_count];
        NSMutableArray<NSValue *> *ids = [NSMutableArray arrayWithCapacity:layer_count];
        for (size_t index = 0; index < layer_count; index++) {
            const SpliceAVLayer *layer = &layers[index];
            SpliceAVTrack *source = track_at(assembly, layer->track);
            bool finite = true;
            for (size_t k = 0; k < 4; k++) finite = finite && isfinite(layer->focus[k]);
            if (source == nil || source.mix != nil || !valid_rect(layer->crop) || !valid_rect(layer->destination) ||
                !isfinite(layer->scale[0]) || !isfinite(layer->scale[1]) || !(layer->scale[0] > 0 && layer->scale[1] > 0) ||
                !finite || !isfinite(layer->source[0]) || !isfinite(layer->source[1]) ||
                !(layer->corner >= 0 && isfinite(layer->corner)) || !(layer->shadow >= 0 && layer->shadow <= 1) ||
                !(layer->cursor_scale > 0 && isfinite(layer->cursor_scale)) ||
                (layer->cursor >= 0 && (NSUInteger)layer->cursor >= assembly.cursors.count) ||
                !(layer->opacity[0] >= 0 && layer->opacity[0] <= 1 &&
                  layer->opacity[1] >= 0 && layer->opacity[1] <= 1)) {
                splice_set_error(error, "Invalid picture layer in a composition instruction.");
                return false;
            }
            [sources addObject:source];
            NSValue *trackID = @(source.track.trackID);
            if (!source.fill && ![ids containsObject:trackID]) [ids addObject:trackID];
        }
        for (size_t index = 0; index < gain_count; index++) {
            const SpliceAVGain *gain = &gains[index];
            SpliceAVTrack *source = track_at(assembly, gain->track);
            if (source.mix == nil || !(gain->gain[0] >= 0 && gain->gain[0] <= 1 &&
                                       gain->gain[1] >= 0 && gain->gain[1] <= 1)) {
                splice_set_error(error, "Invalid audio gain in a composition instruction.");
                return false;
            }
            [source.mix setVolumeRampFromStartVolume:gain->gain[0] toEndVolume:gain->gain[1] timeRange:interval];
        }
        SpliceInstruction *instruction = [SpliceInstruction new];
        instruction.timeRange = interval;
        instruction.enablePostProcessing = NO;
        instruction.containsTweening = YES;
        instruction.requiredSourceTrackIDs = ids;
        instruction.passthroughTrackID = kCMPersistentTrackID_Invalid;
        instruction.layers = [NSData dataWithBytes:layers length:layer_count * sizeof(SpliceAVLayer)];
        instruction.sources = sources;
        instruction.cursors = assembly.cursors ?: @[];
        instruction.canvas = assembly.size;
        instruction.fps = assembly.fps;
        [assembly.instructions addObject:instruction];
        assembly.covered = to;
        return true;
    }
}

static CGImageRef text_bitmap(NSString *text, CGSize size, double font_size, uint32_t rgb_value) {
    size_t width = (size_t)ceil(size.width), height = (size_t)ceil(size.height);
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(NULL, width, height, 8, 0, space,
                                                 kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
    CGColorSpaceRelease(space);
    if (context == NULL) return NULL;
    CGFloat color_components[4];
    rgb(rgb_value, color_components);
    CGColorRef color = CGColorCreateGenericRGB(color_components[0], color_components[1], color_components[2], 1);
    CTFontRef font = CTFontCreateWithName(CFSTR("Helvetica-Bold"), font_size, NULL);
    CTTextAlignment alignment = kCTTextAlignmentCenter;
    CTParagraphStyleSetting settings = {kCTParagraphStyleSpecifierAlignment, sizeof(alignment), &alignment};
    CTParagraphStyleRef paragraph = CTParagraphStyleCreate(&settings, 1);
    NSAttributedString *styled = [[NSAttributedString alloc] initWithString:text attributes:@{
        (__bridge NSString *)kCTFontAttributeName: (__bridge id)font,
        (__bridge NSString *)kCTForegroundColorAttributeName: (__bridge id)color,
        (__bridge NSString *)kCTParagraphStyleAttributeName: (__bridge id)paragraph,
    }];
    CTFramesetterRef setter = CTFramesetterCreateWithAttributedString((__bridge CFAttributedStringRef)styled);
    CGPathRef path = CGPathCreateWithRect(CGRectMake(0, 0, width, height), NULL);
    CTFrameRef frame = CTFramesetterCreateFrame(setter, CFRangeMake(0, 0), path, NULL);
    CTFrameDraw(frame, context);
    CGImageRef image = CGBitmapContextCreateImage(context);
    CFRelease(frame);
    CGPathRelease(path);
    CFRelease(setter);
    CFRelease(paragraph);
    CFRelease(font);
    CGColorRelease(color);
    CGContextRelease(context);
    return image;
}

bool splice_av_text(void *raw, int32_t from, int32_t to, const SpliceAVText *spec, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        NSString *text = [NSString stringWithUTF8String:spec->text];
        if (text == nil || from < 0 || to <= from || to > assembly.end ||
            !valid_rect(spec->bounds) || !isfinite(spec->font) || spec->font <= 0) {
            splice_set_error(error, "Invalid offline title or caption interval.");
            return false;
        }
        if (assembly.text == nil) {
            assembly.text = [CALayer layer];
            assembly.text.frame = CGRectMake(0, 0, assembly.size.width, assembly.size.height);
        }
        SpliceAVTextLayer *layer = [SpliceAVTextLayer layer];
        layer.firstFrame = from;
        layer.endFrame = to;
        layer.fromOpacity = spec->opacity[0];
        layer.toOpacity = spec->opacity[1];
        CGRect bounds = rect(spec->bounds);
        bounds.origin.y = assembly.size.height - bounds.origin.y - bounds.size.height;
        layer.frame = bounds;
        CGImageRef image = text_bitmap(text, bounds.size, spec->font, spec->color);
        if (image == NULL) {
            splice_set_error(error, "Could not rasterize the title or caption.");
            return false;
        }
        layer.contents = (__bridge id)image;
        CGImageRelease(image);
        [assembly.text addSublayer:layer];
        return true;
    }
}

bool splice_av_finish(void *raw, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        if (assembly.covered != assembly.end) {
            splice_set_error(error, "The video instructions do not cover the full timeline.");
            return false;
        }
        AVMutableVideoComposition *video = [AVMutableVideoComposition videoComposition];
        video.customVideoCompositorClass = [SpliceCompositor class];
        video.renderSize = assembly.size;
        video.frameDuration = time_for(assembly, 1);
        video.instructions = assembly.instructions;
        assembly.video = video;
        NSMutableArray *brokered = [NSMutableArray arrayWithCapacity:assembly.instructions.count];
        for (SpliceInstruction *source in assembly.instructions) {
            SpliceInstruction *copy = [SpliceInstruction new];
            copy.timeRange = source.timeRange;
            copy.containsTweening = YES;
            copy.requiredSourceTrackIDs = source.requiredSourceTrackIDs;
            copy.passthroughTrackID = kCMPersistentTrackID_Invalid;
            copy.layers = source.layers;
            copy.sources = source.sources;
            copy.cursors = source.cursors;
            copy.canvas = source.canvas;
            copy.fps = source.fps;
            copy.broker = assembly;
            [brokered addObject:copy];
        }
        assembly.brokered = [video mutableCopy];
        assembly.brokered.instructions = brokered;
        // Export draws text in the compositor; the viewer draws the same evaluator text in PXUI.
        if (assembly.text != nil) {
            NSMutableArray *offline = [NSMutableArray arrayWithCapacity:assembly.instructions.count];
            for (SpliceInstruction *source in assembly.instructions) {
                SpliceInstruction *copy = [SpliceInstruction new];
                copy.timeRange = source.timeRange;
                copy.enablePostProcessing = source.enablePostProcessing;
                copy.containsTweening = source.containsTweening;
                copy.requiredSourceTrackIDs = source.requiredSourceTrackIDs;
                copy.passthroughTrackID = source.passthroughTrackID;
                copy.layers = source.layers;
                copy.sources = source.sources;
                copy.cursors = source.cursors;
                copy.canvas = source.canvas;
                copy.fps = source.fps;
                copy.texts = (NSArray<SpliceAVTextLayer *> *)assembly.text.sublayers;
                [offline addObject:copy];
            }
            assembly.offline = [video mutableCopy];
            assembly.offline.instructions = offline;
        } else {
            assembly.offline = video;
        }
        NSMutableArray *audio = [NSMutableArray array];
        for (SpliceAVTrack *entry in assembly.tracks) {
            if (entry.mix != nil) [audio addObject:entry.mix];
        }
        assembly.audio = [AVMutableAudioMix audioMix];
        assembly.audio.inputParameters = audio;
        return true;
    }
}

bool splice_av_read_frame(void *raw, int32_t frame, uint8_t *rgba, size_t length,
                         bool draw_text, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        size_t width = (size_t)assembly.size.width, height = (size_t)assembly.size.height;
        if (frame < 0 || frame >= assembly.end || length != width * height * 4) {
            splice_set_error(error, "Provide a frame within the timeline and a matching RGBA buffer.");
            return false;
        }
        if (assembly.generator == nil) {
            assembly.generator = [AVAssetImageGenerator assetImageGeneratorWithAsset:assembly.asset];
            assembly.generator.videoComposition = assembly.video;
            assembly.generator.requestedTimeToleranceBefore = kCMTimeZero;
            assembly.generator.requestedTimeToleranceAfter = kCMTimeZero;
        }
        NSError *failure = nil;
        CGImageRef image = [assembly.generator copyCGImageAtTime:time_for(assembly, frame) actualTime:NULL error:&failure];
        if (image == NULL) {
            fail(error, failure, "AVFoundation could not read the requested frame.");
            return false;
        }
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = CGBitmapContextCreate(rgba, width, height, 8, width * 4, color,
                                                     kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
        CGColorSpaceRelease(color);
        if (context == NULL) {
            CGImageRelease(image);
            splice_set_error(error, "Could not map the RGBA frame buffer.");
            return false;
        }
        CGContextSetBlendMode(context, kCGBlendModeCopy);
        CGContextDrawImage(context, CGRectMake(0, 0, width, height), image);
        // ImageGenerator renders the video instructions. Reuse the offline text bitmaps
        // and their sampled opacity so grabs include the titles and captions in the MP4.
        CGContextSetBlendMode(context, kCGBlendModeNormal);
        for (SpliceAVTextLayer *layer in draw_text ? assembly.text.sublayers : nil) {
            if (frame >= layer.firstFrame && frame < layer.endFrame) {
                double progress = (double)(frame - layer.firstFrame) / (layer.endFrame - layer.firstFrame);
                CGContextSetAlpha(context, layer.fromOpacity + (layer.toOpacity - layer.fromOpacity) * progress);
                CGContextDrawImage(context, layer.frame, (__bridge CGImageRef)layer.contents);
            }
        }
        CGContextRelease(context);
        CGImageRelease(image);
        return true;
    }
}

void *splice_av_player(void *raw, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        AVPlayerItem *item = [AVPlayerItem playerItemWithAsset:assembly.asset];
        AVPlayerItemVideoOutput *output = [[AVPlayerItemVideoOutput alloc] initWithPixelBufferAttributes:@{
            (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        }];
        if (item == nil || output == nil) {
            splice_set_error(error, "Could not create AVFoundation's program video output.");
            return NULL;
        }
        item.videoComposition = assembly.brokered;
        item.audioMix = assembly.audio;
        [item addOutput:output];
        AVPlayer *player = [AVPlayer playerWithPlayerItem:item];
        if (player == nil) {
            splice_set_error(error, "Could not create the program player.");
            return NULL;
        }
        SpliceAVPlayer *preview = [SpliceAVPlayer new];
        preview.assembly = assembly;
        preview.player = player;
        preview.output = output;
        preview.surfaces = [NSMutableDictionary dictionary];
        preview.slots = [NSMutableArray arrayWithArray:@[@[], @[], @[]]];
        [player seekToTime:kCMTimeZero toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
        return (__bridge_retained void *)preview;
    }
}

void splice_av_player_play(void *raw, bool playing) {
    @autoreleasepool {
        SpliceAVPlayer *preview = (__bridge SpliceAVPlayer *)raw;
        if (playing) [preview.player play]; else [preview.player pause];
    }
}

bool splice_av_player_ended(void *raw) {
    @autoreleasepool {
        SpliceAVPlayer *preview = (__bridge SpliceAVPlayer *)raw;
        CMTime now = preview.player.currentTime;
        return CMTIME_IS_NUMERIC(now) &&
               CMTimeCompare(now, time_for(preview.assembly, preview.assembly.end)) >= 0;
    }
}

bool splice_av_player_seek(void *raw, int32_t frame) {
    @autoreleasepool {
        SpliceAVPlayer *preview = (__bridge SpliceAVPlayer *)raw;
        if (frame < 0 || frame >= preview.assembly.end) return false;
        [preview.player seekToTime:time_for(preview.assembly, frame)
                   toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
        return true;
    }
}


int32_t splice_av_player_sources(void *raw, void *device_raw, uint32_t slot, SpliceAVFrameSource *sources,
                                 size_t capacity, size_t *count, int32_t *frame, char **error) {
    @autoreleasepool {
        SpliceAVPlayer *preview = (__bridge SpliceAVPlayer *)raw;
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_raw;
        SpliceAVAssembly *assembly = preview.assembly;
        if (slot >= 3 || device == nil || sources == NULL || count == NULL || frame == NULL) {
            splice_set_error(error, "Invalid compositor source request.");
            return -1;
        }
        AVPlayerItem *item = preview.player.currentItem;
        if (item.status == AVPlayerItemStatusFailed) {
            fail(error, item.error, "The program player failed.");
            return -1;
        }
        if (item.status != AVPlayerItemStatusReadyToPlay) return 0;
        CMTime time = CMTimeMinimum(preview.player.currentTime, time_for(assembly, assembly.end - 1));
        if (!CMTIME_IS_NUMERIC(time) || ![preview.output hasNewPixelBufferForItemTime:time]) return 0;
        CMTime display = kCMTimeInvalid;
        CVPixelBufferRef placeholder = [preview.output copyPixelBufferForItemTime:time itemTimeForDisplay:&display];
        if (placeholder != NULL) CFRelease(placeholder);
        if (!CMTIME_IS_NUMERIC(display)) display = time;
        int32_t at = (int32_t)MAX(0, MIN(assembly.end - 1,
            CMTimeConvertScale(display, assembly.fps, kCMTimeRoundingMethod_RoundHalfAwayFromZero).value));
        NSArray *entry = nil;
        @synchronized (assembly) {
            for (NSArray *candidate in assembly.frames.reverseObjectEnumerator) {
                if ([candidate[0] intValue] == at) { entry = candidate; break; }
            }
        }
        if (entry == nil || entry.count - 1 > capacity) return 0;
        preview.slots[slot] = entry;  // retains this frame's buffers until the slot is reused
        for (NSUInteger index = 1; index < entry.count; index++) {
            SpliceAVFrameSource *out = &sources[index - 1];
            *out = (SpliceAVFrameSource){0};
            if (entry[index] == [NSNull null]) continue;
            CVPixelBufferRef pixels = (__bridge CVPixelBufferRef)entry[index];
            IOSurfaceRef surface = CVPixelBufferGetIOSurface(pixels);
            if (surface == NULL || CVPixelBufferGetPixelFormatType(pixels) != kCVPixelFormatType_32BGRA) {
                splice_set_error(error, "A decoded frame is not an IOSurface-backed BGRA buffer.");
                return -1;
            }
            NSNumber *key = @(IOSurfaceGetID(surface));
            id<MTLTexture> texture = preview.surfaces[key];
            size_t width = CVPixelBufferGetWidth(pixels), height = CVPixelBufferGetHeight(pixels);
            if (texture == nil || texture.width != width || texture.height != height) {
                // Decoder pools recycle buffers; drop the cache if a pool keeps turning over.
                if (preview.surfaces.count >= 64) [preview.surfaces removeAllObjects];
                // Viewed as RGBA8 (bytes unchanged) so it copies into OGPU's RGBA textures; the
                // compositor swaps red and blue back when it samples (splice_gpu::Layer::BGRA).
                MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
                    texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:width height:height mipmapped:NO];
                descriptor.usage = MTLTextureUsageShaderRead;
                descriptor.storageMode = MTLStorageModeShared;
                texture = [device newTextureWithDescriptor:descriptor iosurface:surface plane:0];
                if (texture == nil) {
                    splice_set_error(error, "Metal could not wrap a decoded frame.");
                    return -1;
                }
                preview.surfaces[key] = texture;
            }
            out->texture = (__bridge void *)texture;
            out->surface = (uint32_t)key.unsignedIntValue;
            out->width = (uint32_t)width;
            out->height = (uint32_t)height;
        }
        *count = entry.count - 1;
        *frame = at;
        return 1;
    }
}

bool splice_av_blit(void *command_buffer, void *const *sources, void *const *destinations, size_t count,
                    char **error) {
    @autoreleasepool {
        id<MTLCommandBuffer> commands = (__bridge id<MTLCommandBuffer>)command_buffer;
        if (commands == nil || (count > 0 && (sources == NULL || destinations == NULL))) {
            splice_set_error(error, "Invalid frame copy.");
            return false;
        }
        if (count == 0) return true;
        id<MTLBlitCommandEncoder> blit = [commands blitCommandEncoder];
        for (size_t index = 0; index < count; index++) {
            id<MTLTexture> from = (__bridge id<MTLTexture>)sources[index];
            id<MTLTexture> to = (__bridge id<MTLTexture>)destinations[index];
            if (from == nil || to == nil || from.width != to.width || from.height != to.height ||
                from.pixelFormat != to.pixelFormat) {
                [blit endEncoding];
                splice_set_error(error, "A frame copy needs textures of one size and format.");
                return false;
            }
            [blit copyFromTexture:from toTexture:to];
        }
        [blit endEncoding];
        return true;
    }
}

uint8_t *splice_image_rgba(const char *path, uint32_t *width, uint32_t *height, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        CGImageRef image = still_image(url);
        if (image == NULL) {
            splice_set_error(error, "Could not decode the image.");
            return NULL;
        }
        size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
        uint8_t *pixels = calloc(w * h, 4);
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = pixels == NULL ? NULL : CGBitmapContextCreate(pixels, w, h, 8, w * 4, space,
            kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
        CGColorSpaceRelease(space);
        if (context == NULL) {
            free(pixels);
            CGImageRelease(image);
            splice_set_error(error, "Could not allocate image pixels.");
            return NULL;
        }
        CGContextDrawImage(context, CGRectMake(0, 0, w, h), image);
        CGContextRelease(context);
        CGImageRelease(image);
        // Unpremultiply: the compositor blends straight alpha.
        for (size_t i = 0; i < w * h * 4; i += 4) {
            uint8_t a = pixels[i + 3];
            if (a != 0 && a != 255) for (int c = 0; c < 3; c++) pixels[i + c] = (uint8_t)MIN(255, pixels[i + c] * 255 / a);
        }
        *width = (uint32_t)w;
        *height = (uint32_t)h;
        return pixels;
    }
}

void *splice_av_export(void *raw, const char *file, bool hevc, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:file isDirectory:NO relativeToURL:nil];
        NSString *preset = hevc ? AVAssetExportPresetHEVCHighestQuality : AVAssetExportPresetHighestQuality;
        AVAssetExportSession *session = [[AVAssetExportSession alloc] initWithAsset:assembly.asset presetName:preset];
        if (url == nil || ![session.supportedFileTypes containsObject:AVFileTypeMPEG4]) {
            splice_set_error(error, "The native MP4 export preset is unavailable for this composition.");
            return NULL;
        }
        session.outputURL = url;
        session.outputFileType = AVFileTypeMPEG4;
        session.videoComposition = assembly.offline;
        session.audioMix = assembly.audio;
        session.timeRange = CMTimeRangeMake(kCMTimeZero, time_for(assembly, assembly.end));
        SpliceAVExport *job = [SpliceAVExport new];
        job.session = session;
        job.assembly = assembly;
        [session exportAsynchronouslyWithCompletionHandler:^{
            // Retain static picture tracks until AVFoundation completes or acknowledges cancellation.
            (void)job.assembly;
        }];
        return (__bridge_retained void *)job;
    }
}

int32_t splice_av_export_status(void *raw, float *progress, char **error) {
    @autoreleasepool {
        AVAssetExportSession *session = ((__bridge SpliceAVExport *)raw).session;
        *progress = session.progress;
        switch (session.status) {
            case AVAssetExportSessionStatusCompleted: return 1;
            case AVAssetExportSessionStatusCancelled: return 2;
            case AVAssetExportSessionStatusFailed:
                fail(error, session.error, "AVFoundation export failed.");
                return -1;
            default: return 0;
        }
    }
}

void splice_av_export_cancel(void *raw) {
    @autoreleasepool {
        [((__bridge SpliceAVExport *)raw).session cancelExport];
    }
}

void splice_av_export_release(void *raw) {
    @autoreleasepool {
        SpliceAVExport *job = CFBridgingRelease(raw);
        if (job.session.status == AVAssetExportSessionStatusExporting ||
            job.session.status == AVAssetExportSessionStatusWaiting) {
            [job.session cancelExport];
        }
    }
}

@implementation SpliceAVExport
@end

#pragma clang diagnostic pop
