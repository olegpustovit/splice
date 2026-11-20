#include "platform.h"

#import <AppKit/AppKit.h>
#import <Accelerate/Accelerate.h>
#import <AVFoundation/AVFoundation.h>
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

// The one SDR working space, as Recorder's: sRGB values (BT.709 primaries, sRGB transfer) and a
// BT.709 YCbCr matrix. Sources convert into it on decode, program frames hold it, and every movie
// export and picture track is tagged with it, so view.frames, PNG, GIF and the movies agree.
NSDictionary *splice_video_color(void) {
    return @{AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
             AVVideoTransferFunctionKey: AVVideoTransferFunction_IEC_sRGB,
             AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2};
}

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


@interface SpliceAVAssembly : NSObject
@property(nonatomic) CGSize size;
@property(nonatomic) int32_t fps;
@property(nonatomic) int32_t end;
@property(nonatomic) int32_t covered;
@property(nonatomic, strong) AVMutableComposition *asset;
@property(nonatomic, strong) NSMutableArray<SpliceAVTrack *> *tracks;
@property(nonatomic, strong) NSMutableArray *instructions;
// Source buffers per frame from brokered requests, newest last (at most 8). Guarded by @synchronized.
@property(nonatomic, strong) NSMutableArray<NSArray *> *frames;
@property(nonatomic, strong) NSMutableDictionary<NSURL *, AVURLAsset *> *inputs;
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSURL *> *stills;
// Picture movies made from animated images (GIF, APNG): they loop instead of stretching.
@property(nonatomic, strong) NSMutableSet<NSURL *> *animated;
@property(nonatomic, strong) NSURL *scratch;
@property(nonatomic, strong) AVMutableVideoComposition *video;
@property(nonatomic, strong) NSMutableDictionary *surfaces;  // frame grabs' IOSurface views
@property(nonatomic, strong) NSArray *grabbed;
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

// Frame `index` of a picture movie as a +1 pixel buffer (NULL when it cannot be made) and the
// seconds it stays on screen.
typedef CVPixelBufferRef (^SpliceFrameAt)(size_t index, double *duration);

// Encodes `count` frames from `frame_at` as a ProRes 4444 picture movie (alpha kept); a still is
// one frame of 1/30 s that placement stretches. Frames are made one at a time on the encoder's
// queue while this thread waits, so `frame_at` must be safe to call from another thread.
// `unmade` is the error when `frame_at` returns NULL.
static bool write_frames(NSURL *url, size_t count, SpliceFrameAt frame_at, const char *unmade, char **error) {
    double first_duration = 0;
    __block CVPixelBufferRef first = count > 0 ? frame_at(0, &first_duration) : NULL;
    if (first == NULL) {
        splice_set_error(error, unmade);
        return false;
    }
    NSError *failure = nil;
    AVAssetWriter *writer = [[AVAssetWriter alloc] initWithURL:url fileType:AVFileTypeQuickTimeMovie error:&failure];
    if (writer == nil) {
        CFRelease(first);
        fail(error, failure, "Could not open a temporary picture track.");
        return false;
    }
    AVAssetWriterInput *input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:@{
        AVVideoCodecKey: AVVideoCodecTypeAppleProRes4444,
        AVVideoColorPropertiesKey: splice_video_color(),
        AVVideoWidthKey: @(CVPixelBufferGetWidth(first)),
        AVVideoHeightKey: @(CVPixelBufferGetHeight(first)),
    }];
    if (![writer canAddInput:input]) {
        CFRelease(first);
        fail(error, writer.error, "Could not encode a picture track.");
        return false;
    }
    [writer addInput:input];
    AVAssetWriterInputPixelBufferAdaptor *adaptor =
        [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input
                                                                        sourcePixelBufferAttributes:nil];
    if (![writer startWriting]) {
        CFRelease(first);
        fail(error, writer.error, "Could not start the picture encoder.");
        return false;
    }
    [writer startSessionAtSourceTime:kCMTimeZero];
    dispatch_semaphore_t complete = dispatch_semaphore_create(0);
    dispatch_queue_t queue = dispatch_queue_create("splice.still", DISPATCH_QUEUE_SERIAL);
    __block size_t next = 0;
    __block double at = 0;
    __block bool done = false;
    __block bool missing = false;
    [input requestMediaDataWhenReadyOnQueue:queue usingBlock:^{
        if (done) return;
        while (input.readyForMoreMediaData && next < count) {
            double duration = first_duration;
            CVPixelBufferRef pixel = first;
            first = NULL;
            if (next > 0) pixel = frame_at(next, &duration);
            bool appended = pixel != NULL && writer.status == AVAssetWriterStatusWriting &&
                [adaptor appendPixelBuffer:pixel withPresentationTime:CMTimeMakeWithSeconds(at, 600)];
            if (pixel != NULL) CFRelease(pixel);
            if (!appended) {
                missing = pixel == NULL;
                done = true;
                [writer cancelWriting];
                dispatch_semaphore_signal(complete);
                return;
            }
            at += duration;
            next += 1;
        }
        if (next == count) {
            done = true;
            [writer endSessionAtSourceTime:CMTimeMakeWithSeconds(at, 600)];
            [input markAsFinished];
            [writer finishWritingWithCompletionHandler:^{ dispatch_semaphore_signal(complete); }];
        }
    }];
    // Frames are made inside the wait, so it allows for decoding or drawing them too.
    int64_t wait = (int64_t)(3 + count / 10) * (int64_t)NSEC_PER_SEC;
    if (dispatch_semaphore_wait(complete, dispatch_time(DISPATCH_TIME_NOW, wait)) != 0) {
        [writer cancelWriting];
        // Waits out a frame being made, so `frame_at` (and what it borrows) is never called
        // after this returns.
        dispatch_sync(queue, ^{ done = true; });
        if (first != NULL) CFRelease(first);
        splice_set_error(error, "Timed out encoding a temporary picture track.");
        return false;
    }
    if (missing) {
        splice_set_error(error, unmade);
        return false;
    }
    if (writer.status != AVAssetWriterStatusCompleted) {
        fail(error, writer.error, "Could not encode the temporary picture track.");
        return false;
    }
    return true;
}

// Seconds frame `index` of an animated GIF or APNG stays on screen (browsers' 0.1 s for 0).
static double frame_delay(CGImageSourceRef source, size_t index) {
    NSDictionary *properties = CFBridgingRelease(CGImageSourceCopyPropertiesAtIndex(source, index, NULL));
    NSDictionary *gif = properties[(__bridge NSString *)kCGImagePropertyGIFDictionary];
    NSDictionary *png = properties[(__bridge NSString *)kCGImagePropertyPNGDictionary];
    NSNumber *delay = gif[(__bridge NSString *)kCGImagePropertyGIFUnclampedDelayTime]
        ?: gif[(__bridge NSString *)kCGImagePropertyGIFDelayTime]
        ?: png[(__bridge NSString *)kCGImagePropertyAPNGUnclampedDelayTime]
        ?: png[(__bridge NSString *)kCGImagePropertyAPNGDelayTime];
    double seconds = delay.doubleValue;
    return seconds >= 0.011 ? seconds : 0.1;
}

// Every frame of an animated image as a looping picture movie (M23); nil when it has one frame.
static NSURL *animated_movie(SpliceAVAssembly *assembly, NSURL *file, NSURL *url, char **error) {
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)file, NULL);
    if (source == NULL) return nil;
    size_t count = CGImageSourceGetCount(source);
    if (count < 2) {
        CFRelease(source);
        return nil;
    }
    bool ok = write_frames(url, MIN(count, (size_t)2000), ^CVPixelBufferRef(size_t index, double *duration) {
        *duration = frame_delay(source, index);
        CGImageRef image = CGImageSourceCreateImageAtIndex(source, index, NULL);
        if (image == NULL) return NULL;
        CVPixelBufferRef pixel = still_buffer(image, NULL);
        CGImageRelease(image);
        return pixel;
    }, "Could not decode the frames of an animated image.", error);
    CFRelease(source);
    if (!ok) return nil;
    [assembly.animated addObject:url];
    return url;
}

// Where the next picture movie is written, in the assembly's private directory.
static NSURL *scratch_movie(SpliceAVAssembly *assembly, char **error) {
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
    return [assembly.scratch URLByAppendingPathComponent:[NSString stringWithFormat:@"%lu.mov", assembly.stills.count]];
}

static NSURL *still_movie(SpliceAVAssembly *assembly, NSURL *file, const uint32_t colors[2], char **error) {
    NSString *key = file != nil ? file.absoluteString : [NSString stringWithFormat:@"rgb:%u:%u", colors[0], colors[1]];
    NSURL *existing = assembly.stills[key];
    if (existing != nil) return existing;
    NSURL *url = scratch_movie(assembly, error);
    if (url == nil) return nil;
    if (file != nil) {
        NSURL *movie = animated_movie(assembly, file, url, error);
        if (movie != nil) {
            assembly.stills[key] = movie;
            return movie;
        }
        if (error != NULL && *error != NULL) return nil;
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
    bool written = write_frames(url, 1, ^CVPixelBufferRef(size_t index, double *duration) {
        (void)index;
        *duration = 1.0 / 30.0;
        return (CVPixelBufferRef)CFRetain(pixel);
    }, "Could not allocate the still image buffer.", error);
    CFRelease(pixel);
    if (!written) return nil;
    assembly.stills[key] = url;
    return url;
}

bool splice_av_frames(void *raw, const char *path, uint32_t width, uint32_t height, size_t count, double seconds,
                      SpliceAVFill fill, void *context, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        if (path == NULL || fill == NULL || width == 0 || height == 0 || width > 16384 || height > 16384 ||
            count == 0 || !isfinite(seconds) || seconds <= 0) {
            splice_set_error(error, "A drawn picture track needs a path, a size, frames and a frame duration.");
            return false;
        }
        NSURL *file = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        NSString *key = file.absoluteString;
        if (assembly.stills[key] != nil) return true;
        NSURL *url = scratch_movie(assembly, error);
        if (url == nil) return false;
        size_t bytes = (size_t)width * height * 4;
        uint8_t *rgba = malloc(bytes);
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if (rgba == NULL || space == NULL) {
            free(rgba);
            if (space != NULL) CGColorSpaceRelease(space);
            splice_set_error(error, "Could not allocate a drawn picture frame.");
            return false;
        }
        bool ok = write_frames(url, count, ^CVPixelBufferRef(size_t index, double *duration) {
            *duration = seconds;
            if (!fill(context, index, rgba, bytes)) return NULL;
            // Straight alpha in, premultiplied BGRA out: still_buffer draws it like any image.
            CGDataProviderRef provider = CGDataProviderCreateWithData(NULL, rgba, bytes, NULL);
            CGImageRef image = provider != NULL
                ? CGImageCreate(width, height, 8, 32, (size_t)width * 4, space,
                                kCGBitmapByteOrderDefault | kCGImageAlphaLast, provider, NULL, false,
                                kCGRenderingIntentDefault)
                : NULL;
            CGDataProviderRelease(provider);
            if (image == NULL) return NULL;
            CVPixelBufferRef pixel = still_buffer(image, NULL);
            CGImageRelease(image);
            return pixel;
        }, "Could not draw a frame of a vector picture.", error);
        CGColorSpaceRelease(space);
        free(rgba);
        if (!ok) return false;
        assembly.stills[key] = url;
        if (count > 1) [assembly.animated addObject:url];
        return true;
    }
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
    if (spec->kind >= SourceStill && [assembly.animated containsObject:((AVURLAsset *)input.asset).URL]) {
        // An animated image loops at its own pace for as long as the item lasts.
        CMTime at = start;
        while (CMTimeCompare(at, end) < 0) {
            CMTime length = CMTimeMinimum(available.duration, CMTimeSubtract(end, at));
            CMTimeRange range = CMTimeRangeMake(available.start, length);
            if (!insert(destination, input, range, at, length, error)) return false;
            at = CMTimeAdd(at, length);
        }
        return true;
    }
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
        assembly.animated = [NSMutableSet set];
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

bool splice_av_append(void *raw, int32_t index, const SpliceAVSource *spec, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        SpliceAVTrack *entry = track_at(assembly, index);
        if (entry == nil || entry.fill || spec->kind >= SourceStill || spec->path == NULL ||
            spec->timeline[0] < 0 || spec->timeline[0] >= spec->timeline[1] || spec->timeline[1] > assembly.end ||
            !isfinite(spec->speed) || spec->speed <= 0 || !isfinite(spec->source_start) || spec->source_start < 0) {
            splice_set_error(error, "The appended range is invalid.");
            return false;
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:spec->path isDirectory:NO relativeToURL:nil];
        AVURLAsset *asset = assembly.inputs[url];
        if (asset == nil) {
            asset = [AVURLAsset URLAssetWithURL:url options:@{AVURLAssetPreferPreciseDurationAndTimingKey: @YES}];
            assembly.inputs[url] = asset;
        }
        AVMediaType type = spec->kind == SourceAudio ? AVMediaTypeAudio : AVMediaTypeVideo;
        NSArray<AVAssetTrack *> *inputs = [asset tracksWithMediaType:type];
        if (spec->index >= inputs.count) {
            splice_set_error(error, "The appended source track is missing.");
            return false;
        }
        return place(assembly, entry.track, inputs[spec->index], spec, error);
    }
}

uint32_t splice_av_orientation(void *raw, int32_t index) {
    SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
    SpliceAVTrack *entry = track_at(assembly, index);
    if (entry == nil || entry.fill) return 0;
    CGAffineTransform t = entry.orientation;
    if (t.a == 0 && t.b > 0 && t.c < 0 && t.d == 0) return 1;
    if (t.a < 0 && t.b == 0 && t.c == 0 && t.d < 0) return 2;
    if (t.a == 0 && t.b < 0 && t.c > 0 && t.d == 0) return 3;
    return 0;
}

// ── Broker compositor (PLAN.md D1, T12.7) ──
// AVFoundation decodes, times and mixes; OGPU (splice_gpu) draws every pixel. This compositor only
// pins each frame's decoded buffers for Rust to import. Its output buffer carries just the timing.

@interface SpliceInstruction : NSObject <AVVideoCompositionInstruction>
@property(nonatomic) CMTimeRange timeRange;
@property(nonatomic) BOOL enablePostProcessing;
@property(nonatomic) BOOL containsTweening;
@property(nonatomic, strong, nullable) NSArray<NSValue *> *requiredSourceTrackIDs;
@property(nonatomic) CMPersistentTrackID passthroughTrackID;
@property(nonatomic, strong) NSArray<SpliceAVTrack *> *sources;  // one per layer
@property(nonatomic, weak) SpliceAVAssembly *assembly;
@property(nonatomic) int32_t fps;
@end
@implementation SpliceInstruction
@end

static NSDictionary *buffer_attributes(void) {
    return @{(id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
             (id)kCVPixelBufferIOSurfacePropertiesKey: @{}};
}

@interface SpliceCompositor : NSObject <AVVideoCompositing>
@end

@implementation SpliceCompositor
- (NSDictionary *)sourcePixelBufferAttributes {
    return buffer_attributes();
}

- (NSDictionary *)requiredPixelBufferAttributesForRenderContext {
    return buffer_attributes();
}

- (void)renderContextChanged:(AVVideoCompositionRenderContext *)context {
    (void)context;
}

- (void)cancelAllPendingVideoCompositionRequests {
}

- (void)startVideoCompositionRequest:(AVAsynchronousVideoCompositionRequest *)request {
    @autoreleasepool {
        SpliceInstruction *instruction = (SpliceInstruction *)request.videoCompositionInstruction;
        SpliceAVAssembly *assembly = instruction.assembly;
        CVPixelBufferRef output = [request.renderContext newPixelBuffer];
        if (output == NULL || assembly == nil) {
            if (output != NULL) CVPixelBufferRelease(output);
            [request finishWithError:[NSError errorWithDomain:@"Splice" code:1 userInfo:nil]];
            return;
        }
        NSMutableArray *entry = [NSMutableArray arrayWithCapacity:instruction.sources.count + 1];
        [entry addObject:@((int32_t)llround(CMTimeGetSeconds(request.compositionTime) * instruction.fps))];
        for (SpliceAVTrack *source in instruction.sources) {
            CVPixelBufferRef pixels = source.fill ? NULL : [request sourceFrameByTrackID:source.track.trackID];
            [entry addObject:pixels != NULL ? (__bridge id)pixels : (id)[NSNull null]];
        }
        @synchronized (assembly) {
            if (assembly.frames == nil) assembly.frames = [NSMutableArray array];
            [assembly.frames addObject:entry];
            // Bounded: decoders only lend a few buffers, and each entry holds some.
            while (assembly.frames.count > 12) [assembly.frames removeObjectAtIndex:0];
        }
        [request finishWithComposedVideoFrame:output];
        CVPixelBufferRelease(output);
    }
}
@end

// Takes the pinned sources for `frame`, dropping it and everything before it.
static NSArray *take_frame(SpliceAVAssembly *assembly, int32_t frame) {
    @synchronized (assembly) {
        NSArray *found = nil;
        NSMutableIndexSet *done = [NSMutableIndexSet indexSet];
        for (NSUInteger index = 0; index < assembly.frames.count; index++) {
            int32_t at = [assembly.frames[index][0] intValue];
            if (at == frame) found = assembly.frames[index];
            if (at <= frame) [done addIndex:index];
        }
        [assembly.frames removeObjectsAtIndexes:done];
        return found;
    }
}

// RGBA8 views of the entry's decoded BGRA IOSurfaces on `device`, cached per surface. The bytes are
// unchanged: the compositor swaps red and blue back when it samples (splice_gpu::Layer::BGRA).
static int64_t fill_sources(NSArray *entry, id<MTLDevice> device, NSMutableDictionary *surfaces,
                            SpliceAVFrameSource *sources, size_t capacity, char **error) {
    if (entry.count - 1 > capacity) {
        splice_set_error(error, "A frame has more layers than the compositor accepts.");
        return -1;
    }
    // Decoder pools recycle buffers; drop the cache if a pool keeps turning over. Only here,
    // before this frame hands out texture pointers: evicting mid-frame freed textures already
    // given to earlier layers (a dissolve under an overlay crashed in objc_retain).
    if (surfaces.count >= 64) [surfaces removeAllObjects];
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
        id<MTLTexture> texture = surfaces[key];
        size_t width = CVPixelBufferGetWidth(pixels), height = CVPixelBufferGetHeight(pixels);
        if (texture == nil || texture.width != width || texture.height != height || texture.device != device) {
            MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
                texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:width height:height mipmapped:NO];
            descriptor.usage = MTLTextureUsageShaderRead;
            descriptor.storageMode = MTLStorageModeShared;
            texture = [device newTextureWithDescriptor:descriptor iosurface:surface plane:0];
            if (texture == nil) {
                splice_set_error(error, "Metal could not wrap a decoded frame.");
                return -1;
            }
            surfaces[key] = texture;
        }
        out->texture = (__bridge void *)texture;
        out->surface = (uint32_t)key.unsignedIntValue;
        out->width = (uint32_t)width;
        out->height = (uint32_t)height;
    }
    return (int64_t)entry.count - 1;
}

bool splice_av_range(void *raw, int32_t from, int32_t to, const int32_t *tracks, size_t layer_count,
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
            SpliceAVTrack *source = track_at(assembly, tracks[index]);
            if (source == nil || source.mix != nil) {
                splice_set_error(error, "A picture layer names a missing or audio track.");
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
        instruction.containsTweening = YES;
        instruction.requiredSourceTrackIDs = ids;
        instruction.passthroughTrackID = kCMPersistentTrackID_Invalid;
        instruction.sources = sources;
        instruction.assembly = assembly;
        instruction.fps = assembly.fps;
        [assembly.instructions addObject:instruction];
        assembly.covered = to;
        return true;
    }
}

static CTFontRef styled_font(const SpliceTextStyle *style, double size) {
    NSString *name = style->font == NULL ? @"" : [NSString stringWithUTF8String:style->font];
    static const CGFloat weights[6] = {0.0, 0.23, 0.3, 0.4, 0.56, 0.62};
    CGFloat weight = weights[MIN(style->weight, 5u)];
    NSFont *font = nil;
    if (name.length == 0 || [name hasPrefix:@"SF Pro"] || [name isEqualToString:@"System"]) {
        font = [NSFont systemFontOfSize:size weight:weight];
    } else {
        NSFontDescriptor *descriptor = [NSFontDescriptor fontDescriptorWithFontAttributes:@{
            NSFontFamilyAttribute: name,
            NSFontTraitsAttribute: @{NSFontWeightTrait: @(weight)},
        }];
        font = [NSFont fontWithDescriptor:descriptor size:size] ?: [NSFont fontWithName:name size:size];
    }
    if (font == nil) font = [NSFont systemFontOfSize:size weight:weight];
    return (CTFontRef)CFBridgingRetain(font);
}

// The attributed text for one pass: `outline` strokes only (twice the visible width, since a
// stroke is centered on the glyph edge and the fill pass covers its inner half); otherwise
// fill only, with `active` in the active color. Ranges outside `visible` keep their layout but
// draw nothing.
static NSAttributedString *styled_text(NSString *text, CTFontRef font, CTParagraphStyleRef paragraph,
                                       const SpliceTextStyle *style, CGColorRef color, CGColorRef stroke,
                                       CGColorRef clear, BOOL outline, NSRange visible, NSRange active,
                                       CGColorRef active_color) {
    NSMutableDictionary *attributes = [@{
        (__bridge NSString *)kCTFontAttributeName: (__bridge id)font,
        (__bridge NSString *)kCTParagraphStyleAttributeName: (__bridge id)paragraph,
    } mutableCopy];
    if (outline) {
        attributes[(__bridge NSString *)kCTForegroundColorAttributeName] = (__bridge id)clear;
        attributes[(__bridge NSString *)kCTStrokeWidthAttributeName] = @(style->stroke_width * 200.0);
        attributes[(__bridge NSString *)kCTStrokeColorAttributeName] = (__bridge id)stroke;
    } else {
        attributes[(__bridge NSString *)kCTForegroundColorAttributeName] = (__bridge id)color;
    }
    NSMutableAttributedString *styled = [[NSMutableAttributedString alloc] initWithString:text attributes:attributes];
    if (!outline && active.location != NSNotFound) {
        [styled addAttribute:(__bridge NSString *)kCTForegroundColorAttributeName value:(__bridge id)active_color
                       range:active];
    }
    if (visible.location != NSNotFound) {
        NSDictionary *hidden = @{(__bridge NSString *)kCTForegroundColorAttributeName: (__bridge id)clear,
                                 (__bridge NSString *)kCTStrokeColorAttributeName: (__bridge id)clear};
        if (visible.location > 0) [styled addAttributes:hidden range:NSMakeRange(0, visible.location)];
        NSUInteger end = NSMaxRange(visible);
        if (end < text.length) [styled addAttributes:hidden range:NSMakeRange(end, text.length - end)];
    }
    return styled;
}

static CGImageRef text_bitmap(NSString *text, CGSize size, double font_size, const SpliceTextStyle *style,
                              NSRange visible, NSRange active, uint32_t active_rgb) {
    size_t width = (size_t)ceil(size.width), height = (size_t)ceil(size.height);
    CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGContextRef context = CGBitmapContextCreate(NULL, width, height, 8, 0, space,
                                                 kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
    CGColorSpaceRelease(space);
    if (context == NULL) return NULL;
    CGFloat components[4];
    rgb(style->color, components);
    CGColorRef color = CGColorCreateGenericRGB(components[0], components[1], components[2], 1);
    CGColorRef clear = CGColorCreateGenericRGB(0, 0, 0, 0);
    rgb(style->stroke, components);
    CGColorRef stroke = CGColorCreateGenericRGB(components[0], components[1], components[2], 1);
    rgb(active_rgb, components);
    CGColorRef active_color = CGColorCreateGenericRGB(components[0], components[1], components[2], 1);
    CTTextAlignment alignment = style->align == 0 ? kCTTextAlignmentLeft
                              : style->align == 2 ? kCTTextAlignmentRight : kCTTextAlignmentCenter;
    CGFloat multiple = style->line_height > 0 ? style->line_height / 1.15 : 1.0;
    CTParagraphStyleSetting settings[2] = {
        {kCTParagraphStyleSpecifierAlignment, sizeof(alignment), &alignment},
        {kCTParagraphStyleSpecifierLineHeightMultiple, sizeof(multiple), &multiple},
    };
    CTParagraphStyleRef paragraph = CTParagraphStyleCreate(settings, 2);
    CGFloat pad = style->boxed ? font_size * 0.35 : 0;
    // The outline reaches stroke_width × font past the glyphs on every side.
    CGFloat inset = style->stroke_width > 0 ? style->stroke_width * font_size : 0;
    CGSize room = CGSizeMake(MAX(width - 2 * pad - 2 * inset, 1), MAX(height - pad - 2 * inset, 1));
    // Shrink to fit instead of dropping lines that do not fit (T36.5, T42.3).
    CTFontRef font = styled_font(style, font_size);
    for (int attempt = 0; attempt < 8; attempt++) {
        NSAttributedString *probe = styled_text(text, font, paragraph, style, color, stroke, clear, NO,
                                                NSMakeRange(NSNotFound, 0), NSMakeRange(NSNotFound, 0), color);
        CTFramesetterRef measure = CTFramesetterCreateWithAttributedString((__bridge CFAttributedStringRef)probe);
        // Without wrapping, each line is measured at its natural width and must fit the box.
        CGFloat across = style->wrap ? room.width : CGFLOAT_MAX;
        CGSize fit = CTFramesetterSuggestFrameSizeWithConstraints(measure, CFRangeMake(0, 0), NULL,
                                                                  CGSizeMake(across, CGFLOAT_MAX), NULL);
        CFRelease(measure);
        if (fit.height <= room.height + 0.5 && fit.width <= room.width + 0.5) break;
        CFRelease(font);
        CGFloat ratio = MIN(room.height / fit.height, room.width / MAX(fit.width, 1));
        font_size *= MAX(0.5, ratio * 0.98);
        font = styled_font(style, font_size);
    }
    NSAttributedString *fill = styled_text(text, font, paragraph, style, color, stroke, clear, NO, visible, active,
                                           active_color);
    CTFramesetterRef setter = CTFramesetterCreateWithAttributedString((__bridge CFAttributedStringRef)fill);
    if (style->boxed && visible.location == NSNotFound) {
        CGSize fit = CTFramesetterSuggestFrameSizeWithConstraints(setter, CFRangeMake(0, 0), NULL,
                                                                  CGSizeMake(width - 2 * pad, height), NULL);
        CGFloat x = style->align == 0 ? 0 : style->align == 2 ? width - fit.width - 2 * pad : (width - fit.width) / 2 - pad;
        CGRect box = CGRectMake(MAX(x, 0), height - fit.height - 2 * pad, MIN(fit.width + 2 * pad, width),
                                fit.height + 2 * pad);
        CGFloat radius = style->boxed == 2 ? MIN(box.size.height / 2, font_size * 0.4) : 0;
        CGPathRef rounded = CGPathCreateWithRoundedRect(box, radius, radius, NULL);
        rgb(style->box_color, components);
        CGContextSetRGBFillColor(context, components[0], components[1], components[2], 1);
        CGContextAddPath(context, rounded);
        CGContextFillPath(context);
        CGPathRelease(rounded);
    }
    if (style->shadow > 0) {
        CGColorRef shade = CGColorCreateGenericRGB(0, 0, 0, MIN(1.0, style->shadow));
        CGContextSetShadowWithColor(context, CGSizeMake(0, -font_size * 0.05), font_size * 0.15, shade);
        CGColorRelease(shade);
    }
    CGPathRef path = CGPathCreateWithRect(CGRectMake(pad + inset, inset, room.width, room.height), NULL);
    if (style->stroke_width > 0) {
        // Stroke under fill (H8): the outline pass casts the shadow, the fill draws on top.
        NSAttributedString *outline = styled_text(text, font, paragraph, style, color, stroke, clear, YES, visible,
                                                  NSMakeRange(NSNotFound, 0), active_color);
        CTFramesetterRef outliner = CTFramesetterCreateWithAttributedString((__bridge CFAttributedStringRef)outline);
        CTFrameRef under = CTFramesetterCreateFrame(outliner, CFRangeMake(0, 0), path, NULL);
        CTFrameDraw(under, context);
        CFRelease(under);
        CFRelease(outliner);
        CGContextSetShadowWithColor(context, CGSizeZero, 0, NULL);
    }
    CTFrameRef frame = CTFramesetterCreateFrame(setter, CFRangeMake(0, 0), path, NULL);
    CTFrameDraw(frame, context);
    CGImageRef image = CGBitmapContextCreateImage(context);
    CFRelease(frame);
    CGPathRelease(path);
    CFRelease(setter);
    CFRelease(paragraph);
    CFRelease(font);
    CGColorRelease(color);
    CGColorRelease(clear);
    CGColorRelease(stroke);
    CGColorRelease(active_color);
    CGContextRelease(context);
    return image;
}

uint8_t *splice_text_rgba(const char *text, uint32_t width, uint32_t height, double font,
                          const SpliceTextStyle *style, int64_t visible_from, int64_t visible_to,
                          int64_t active_from, int64_t active_to, uint32_t active_color, char **error) {
    @autoreleasepool {
        NSString *string = [NSString stringWithUTF8String:text];
        if (string == nil || width == 0 || height == 0 || width > 16384 || height > 16384 ||
            !isfinite(font) || font <= 0 || style == NULL) {
            splice_set_error(error, "Invalid text raster.");
            return NULL;
        }
        NSRange visible = NSMakeRange(NSNotFound, 0);
        if (visible_from >= 0 && visible_to >= visible_from && (NSUInteger)visible_to <= string.length) {
            visible = NSMakeRange((NSUInteger)visible_from, (NSUInteger)(visible_to - visible_from));
        }
        NSRange active = NSMakeRange(NSNotFound, 0);
        if (active_from >= 0 && active_to > active_from && (NSUInteger)active_to <= string.length) {
            active = NSMakeRange((NSUInteger)active_from, (NSUInteger)(active_to - active_from));
        }
        CGImageRef image = text_bitmap(string, CGSizeMake(width, height), font, style, visible, active, active_color);
        uint8_t *pixels = image == NULL ? NULL : calloc((size_t)width * height, 4);
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = pixels == NULL ? NULL : CGBitmapContextCreate(pixels, width, height, 8, width * 4,
            space, kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast);
        CGColorSpaceRelease(space);
        if (context == NULL) {
            free(pixels);
            if (image != NULL) CGImageRelease(image);
            splice_set_error(error, "Could not rasterize the text.");
            return NULL;
        }
        CGContextDrawImage(context, CGRectMake(0, 0, width, height), image);
        CGContextRelease(context);
        CGImageRelease(image);
        for (size_t i = 0; i < (size_t)width * height * 4; i += 4) {
            uint8_t a = pixels[i + 3];
            if (a != 0 && a != 255) for (int c = 0; c < 3; c++) pixels[i + c] = (uint8_t)MIN(255, pixels[i + c] * 255 / a);
        }
        return pixels;
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
        // Work in the sRGB working space (what export tags): Rec. 709 and wide-color sources
        // such as P3 phone video convert on decode instead of passing their values through
        // (PLAN.md T15.2).
        NSDictionary *color = splice_video_color();
        video.colorPrimaries = color[AVVideoColorPrimariesKey];
        video.colorTransferFunction = color[AVVideoTransferFunctionKey];
        video.colorYCbCrMatrix = color[AVVideoYCbCrMatrixKey];
        video.instructions = assembly.instructions;
        assembly.video = video;
        NSMutableArray *audio = [NSMutableArray array];
        for (SpliceAVTrack *entry in assembly.tracks) {
            if (entry.mix != nil) [audio addObject:entry.mix];
        }
        assembly.audio = [AVMutableAudioMix audioMix];
        assembly.audio.inputParameters = audio;
        return true;
    }
}

int32_t splice_av_grab(void *raw, void *device_raw, int32_t frame, SpliceAVFrameSource *sources, size_t capacity,
                       size_t *count, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_raw;
        if (frame < 0 || frame >= assembly.end || device == nil || sources == NULL || count == NULL) {
            splice_set_error(error, "Provide a frame within the timeline and a device.");
            return -1;
        }
        if (assembly.generator == nil) {
            assembly.generator = [AVAssetImageGenerator assetImageGeneratorWithAsset:assembly.asset];
            assembly.generator.videoComposition = assembly.video;
            assembly.generator.requestedTimeToleranceBefore = kCMTimeZero;
            assembly.generator.requestedTimeToleranceAfter = kCMTimeZero;
            assembly.surfaces = [NSMutableDictionary dictionary];
        }
        NSError *failure = nil;
        CGImageRef image = [assembly.generator copyCGImageAtTime:time_for(assembly, frame) actualTime:NULL error:&failure];
        if (image == NULL) {
            fail(error, failure, "AVFoundation could not read the requested frame.");
            return -1;
        }
        CGImageRelease(image);
        NSArray *entry = take_frame(assembly, frame);
        if (entry == nil) {
            splice_set_error(error, "The compositor did not deliver the requested frame.");
            return -1;
        }
        assembly.grabbed = entry;  // keeps the sources until the next grab
        int64_t filled = fill_sources(entry, device, assembly.surfaces, sources, capacity, error);
        if (filled < 0) return -1;
        *count = (size_t)filled;
        return 1;
    }
}

void *splice_av_player(void *raw, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        AVPlayerItem *item = [AVPlayerItem playerItemWithAsset:assembly.asset];
        AVPlayerItemVideoOutput *output = [[AVPlayerItemVideoOutput alloc] initWithPixelBufferAttributes:buffer_attributes()];
        if (item == nil || output == nil) {
            splice_set_error(error, "Could not create AVFoundation's program video output.");
            return NULL;
        }
        item.videoComposition = assembly.video;
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
        NSArray *entry = take_frame(assembly, at);
        if (entry == nil) return 0;
        preview.slots[slot] = entry;  // retains this frame's buffers until the slot is reused
        int64_t filled = fill_sources(entry, device, preview.surfaces, sources, capacity, error);
        if (filled < 0) return -1;
        *count = (size_t)filled;
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

// ── Export: AVAssetReader decodes through the broker, OGPU draws, AVAssetWriter encodes ──

@interface SpliceAVReader : NSObject
@property(nonatomic, strong) SpliceAVAssembly *assembly;
@property(nonatomic, strong) AVAssetReader *reader;
@property(nonatomic, strong) AVAssetReaderVideoCompositionOutput *video;
@property(nonatomic, strong) AVAssetReaderAudioMixOutput *audio;
@property(nonatomic, strong) NSMutableDictionary *surfaces;
@property(nonatomic, strong) NSArray *held;
@end
@implementation SpliceAVReader
@end

void *splice_av_reader(void *raw, int32_t from, int32_t to, bool with_audio, char **error) {
    @autoreleasepool {
        SpliceAVAssembly *assembly = (__bridge SpliceAVAssembly *)raw;
        NSError *failure = nil;
        AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:assembly.asset error:&failure];
        if (reader == nil) {
            fail(error, failure, "AVFoundation could not read the composition.");
            return NULL;
        }
        if (to > from && from >= 0) {
            reader.timeRange = CMTimeRangeFromTimeToTime(time_for(assembly, from), time_for(assembly, to));
        }
        NSArray *videoTracks = [assembly.asset tracksWithMediaType:AVMediaTypeVideo];
        AVAssetReaderVideoCompositionOutput *video =
            [AVAssetReaderVideoCompositionOutput assetReaderVideoCompositionOutputWithVideoTracks:videoTracks
                                                                                    videoSettings:buffer_attributes()];
        video.videoComposition = assembly.video;
        video.alwaysCopiesSampleData = NO;
        if (![reader canAddOutput:video]) {
            splice_set_error(error, "The composition's video cannot be read.");
            return NULL;
        }
        [reader addOutput:video];
        AVAssetReaderAudioMixOutput *audio = nil;
        NSArray *audioTracks = with_audio ? [assembly.asset tracksWithMediaType:AVMediaTypeAudio] : @[];
        if (audioTracks.count > 0) {
            audio = [AVAssetReaderAudioMixOutput assetReaderAudioMixOutputWithAudioTracks:audioTracks audioSettings:@{
                AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
                AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES, AVLinearPCMIsNonInterleaved: @NO,
                AVLinearPCMIsBigEndianKey: @NO}];
            audio.audioMix = assembly.audio;
            if (![reader canAddOutput:audio]) {
                splice_set_error(error, "The composition's audio cannot be read.");
                return NULL;
            }
            [reader addOutput:audio];
        }
        if (![reader startReading]) {
            fail(error, reader.error, "AVFoundation could not start reading the composition.");
            return NULL;
        }
        SpliceAVReader *out = [SpliceAVReader new];
        out.assembly = assembly;
        out.reader = reader;
        out.video = video;
        out.audio = audio;
        out.surfaces = [NSMutableDictionary dictionary];
        return (__bridge_retained void *)out;
    }
}

int32_t splice_av_reader_next(void *raw, void *device_raw, SpliceAVFrameSource *sources, size_t capacity,
                              size_t *count, int32_t *frame, char **error) {
    @autoreleasepool {
        SpliceAVReader *reader = (__bridge SpliceAVReader *)raw;
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_raw;
        CMSampleBufferRef sample = [reader.video copyNextSampleBuffer];
        if (sample == NULL) {
            if (reader.reader.status == AVAssetReaderStatusFailed) {
                fail(error, reader.reader.error, "AVFoundation stopped reading the composition.");
                return -1;
            }
            return 0;
        }
        CMTime at = CMSampleBufferGetPresentationTimeStamp(sample);
        CFRelease(sample);
        int32_t index = (int32_t)llround(CMTimeGetSeconds(at) * reader.assembly.fps);
        NSArray *entry = take_frame(reader.assembly, index);
        if (entry == nil) {
            splice_set_error(error, "The compositor did not deliver a frame being exported.");
            return -1;
        }
        reader.held = entry;  // the GPU copy of this frame completes before the next call
        int64_t filled = fill_sources(entry, device, reader.surfaces, sources, capacity, error);
        if (filled < 0) return -1;
        *count = (size_t)filled;
        *frame = index;
        return 1;
    }
}

void splice_av_reader_release(void *raw) {
    @autoreleasepool {
        SpliceAVReader *reader = (__bridge_transfer SpliceAVReader *)raw;
        if (reader.reader.status == AVAssetReaderStatusReading) [reader.reader cancelReading];
    }
}

@interface SpliceAVWriter : NSObject
@property(nonatomic, strong) SpliceAVReader *reader;
@property(nonatomic, strong) AVAssetWriter *writer;
@property(nonatomic, strong) AVAssetWriterInput *video;
@property(nonatomic, strong) AVAssetWriterInputPixelBufferAdaptor *adaptor;
@property(nonatomic, strong) AVAssetWriterInput *audio;
@property(nonatomic, strong) dispatch_queue_t queue;
@property(nonatomic, strong) dispatch_semaphore_t audioDone;
@property(nonatomic, strong) NSMutableDictionary *surfaces;
@property(nonatomic, strong) NSURL *url;
@property(nonatomic) CVPixelBufferRef pending;
@property(nonatomic) int32_t fps;
@end
@implementation SpliceAVWriter
- (void)dealloc {
    if (_pending != NULL) CVPixelBufferRelease(_pending);
}
@end

// A 3GPP text format with the extensions writers require (font table, default style and box).
static CMFormatDescriptionRef text_format(void) {
    NSDictionary *clear = @{(id)kCMTextFormatDescriptionColor_Red: @0, (id)kCMTextFormatDescriptionColor_Green: @0,
                            (id)kCMTextFormatDescriptionColor_Blue: @0, (id)kCMTextFormatDescriptionColor_Alpha: @255};
    NSDictionary *extensions = @{
        (id)kCMTextFormatDescriptionExtension_DisplayFlags: @0,
        (id)kCMTextFormatDescriptionExtension_BackgroundColor: clear,
        (id)kCMTextFormatDescriptionExtension_DefaultTextBox: @{
            (id)kCMTextFormatDescriptionRect_Top: @0, (id)kCMTextFormatDescriptionRect_Left: @0,
            (id)kCMTextFormatDescriptionRect_Bottom: @0, (id)kCMTextFormatDescriptionRect_Right: @0},
        (id)kCMTextFormatDescriptionExtension_DefaultStyle: @{
            (id)kCMTextFormatDescriptionStyle_StartChar: @0, (id)kCMTextFormatDescriptionStyle_EndChar: @0,
            (id)kCMTextFormatDescriptionStyle_Font: @1, (id)kCMTextFormatDescriptionStyle_FontFace: @0,
            (id)kCMTextFormatDescriptionStyle_FontSize: @12,
            (id)kCMTextFormatDescriptionStyle_ForegroundColor: @{
                (id)kCMTextFormatDescriptionColor_Red: @255, (id)kCMTextFormatDescriptionColor_Green: @255,
                (id)kCMTextFormatDescriptionColor_Blue: @255, (id)kCMTextFormatDescriptionColor_Alpha: @255}},
        (id)kCMTextFormatDescriptionExtension_HorizontalJustification: @0,
        (id)kCMTextFormatDescriptionExtension_VerticalJustification: @0,
        (id)kCMTextFormatDescriptionExtension_FontTable: @{@"1": @"Sans-Serif"},
    };
    CMFormatDescriptionRef format = NULL;
    CMFormatDescriptionCreate(NULL, kCMMediaType_Text, kCMTextFormatType_3GText,
                              (__bridge CFDictionaryRef)extensions, &format);
    return format;
}

// A 3GPP text sample: 16-bit big-endian length, then UTF-8.
static CMSampleBufferRef text_sample(CMFormatDescriptionRef format, NSString *text, CMTime start, CMTime duration) {
    NSData *utf8 = [text dataUsingEncoding:NSUTF8StringEncoding];
    size_t length = 2 + utf8.length;
    uint8_t *bytes = malloc(length);
    bytes[0] = (uint8_t)(utf8.length >> 8);
    bytes[1] = (uint8_t)(utf8.length & 0xff);
    memcpy(bytes + 2, utf8.bytes, utf8.length);
    CMBlockBufferRef block = NULL;
    CMBlockBufferCreateWithMemoryBlock(NULL, bytes, length, kCFAllocatorMalloc, NULL, 0, length, 0, &block);
    CMSampleTimingInfo timing = {duration, start, kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    CMSampleBufferCreate(NULL, block, true, NULL, NULL, format, 1, 1, &timing, 1, &length, &sample);
    CFRelease(block);
    return sample;
}

void *splice_av_writer(void *reader_raw, const char *file, const SpliceExportSettings *settings, char **error) {
    @autoreleasepool {
        SpliceAVReader *reader = (__bridge SpliceAVReader *)reader_raw;
        SpliceAVAssembly *assembly = reader.assembly;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:file isDirectory:NO relativeToURL:nil];
        uint32_t codec = settings->codec;
        bool video_out = codec <= SpliceCodecHEVCAlpha;
        AVFileType type = codec == SpliceCodecAAC ? AVFileTypeAppleM4A
                        : codec == SpliceCodecWAV ? AVFileTypeWAVE
                        : (codec >= SpliceCodecProRes422 ? AVFileTypeQuickTimeMovie : AVFileTypeMPEG4);
        NSError *failure = nil;
        AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:url fileType:type error:&failure];
        if (writer == nil) {
            fail(error, failure, "AVFoundation could not create the export file.");
            return NULL;
        }
        AVAssetWriterInput *video = nil;
        AVAssetWriterInputPixelBufferAdaptor *adaptor = nil;
        // export.start {fps}: frames are appended at the output rate (the caller samples them).
        int32_t fps = settings->fps > 0 ? (int32_t)settings->fps : assembly.fps;
        if (video_out) {
            double pixels = assembly.size.width * assembly.size.height;
            bool hevc = codec == SpliceCodecHEVC || codec == SpliceCodecHEVCAlpha;
            AVVideoCodecType name = codec == SpliceCodecHEVC ? AVVideoCodecTypeHEVC
                                  : codec == SpliceCodecProRes422 ? AVVideoCodecTypeAppleProRes422HQ
                                  : codec == SpliceCodecProRes4444 ? AVVideoCodecTypeAppleProRes4444
                                  : codec == SpliceCodecHEVCAlpha ? AVVideoCodecTypeHEVCWithAlpha
                                  : AVVideoCodecTypeH264;
            NSMutableDictionary *output = [@{
                AVVideoCodecKey: name,
                AVVideoColorPropertiesKey: splice_video_color(),
                AVVideoWidthKey: @(assembly.size.width), AVVideoHeightKey: @(assembly.size.height),
            } mutableCopy];
            if (codec <= SpliceCodecHEVC || codec == SpliceCodecHEVCAlpha) {
                // About the "highest quality" export preset's rate: 22 Mbps H.264, 16 Mbps HEVC at 1080p30.
                double rate = settings->bitrate > 0 ? settings->bitrate : pixels * fps * (hevc ? 0.25 : 0.35);
                output[AVVideoCompressionPropertiesKey] = @{AVVideoAverageBitRateKey: @((long long)rate),
                                                            AVVideoExpectedSourceFrameRateKey: @(fps)};
            }
            video = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:output];
            video.expectsMediaDataInRealTime = NO;
            adaptor = [AVAssetWriterInputPixelBufferAdaptor
                assetWriterInputPixelBufferAdaptorWithAssetWriterInput:video sourcePixelBufferAttributes:@{
                    (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
                    (id)kCVPixelBufferWidthKey: @(assembly.size.width), (id)kCVPixelBufferHeightKey: @(assembly.size.height),
                    (id)kCVPixelBufferIOSurfacePropertiesKey: @{}}];
            if (![writer canAddInput:video]) {
                splice_set_error(error, "The export cannot take this video format.");
                return NULL;
            }
            [writer addInput:video];
        }
        AVAssetWriterInput *audio = nil;
        if (reader.audio != nil) {
            NSDictionary *audioSettings = codec == SpliceCodecWAV
                ? @{AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
                    AVLinearPCMBitDepthKey: @16, AVLinearPCMIsFloatKey: @NO, AVLinearPCMIsBigEndianKey: @NO,
                    AVLinearPCMIsNonInterleaved: @NO}
                : (codec >= SpliceCodecProRes422 && codec <= SpliceCodecProRes4444
                    ? @{AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
                        AVLinearPCMBitDepthKey: @24, AVLinearPCMIsFloatKey: @NO, AVLinearPCMIsBigEndianKey: @NO,
                        AVLinearPCMIsNonInterleaved: @NO}
                    : @{AVFormatIDKey: @(kAudioFormatMPEG4AAC), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
                        AVEncoderBitRateKey: @(codec == SpliceCodecAAC ? 256000 : 192000)});
            audio = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeAudio outputSettings:audioSettings];
            audio.expectsMediaDataInRealTime = NO;
            if (![writer canAddInput:audio]) {
                splice_set_error(error, "The export cannot take this audio format.");
                return NULL;
            }
            [writer addInput:audio];
        } else if (!video_out) {
            splice_set_error(error, "The timeline has no audio to export.");
            return NULL;
        }
        // Chapters: a 3GPP text track the video names as its chapter list (QuickTime and AVAsset
        // read chapters from it).
        AVAssetWriterInput *chapters = nil;
        NSMutableArray<NSArray *> *marks = [NSMutableArray array];
        if (video_out && settings->chapters != NULL && settings->chapters[0] != 0) {
            for (NSString *line in [[NSString stringWithUTF8String:settings->chapters] componentsSeparatedByString:@"\n"]) {
                NSArray *parts = [line componentsSeparatedByString:@"\t"];
                if (parts.count == 2) [marks addObject:@[@([parts[0] intValue]), parts[1]]];
            }
            CMFormatDescriptionRef format = text_format();
            chapters = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeText outputSettings:nil
                                                        sourceFormatHint:format];
            if (format != NULL) CFRelease(format);
            chapters.languageCode = @"eng";
            chapters.extendedLanguageTag = @"en";
            // Chapter samples are all written before the frames; interleaved with the video, the
            // text input stopped taking them after the first few and the export hung.
            chapters.mediaDataLocation = AVAssetWriterInputMediaDataLocationBeforeMainMediaDataNotInterleaved;
            if (marks.count > 0 && [writer canAddInput:chapters]) {
                [writer addInput:chapters];
                [video addTrackAssociationWithTrackOfInput:chapters type:AVTrackAssociationTypeChapterList];
            } else {
                chapters = nil;
            }
        }
        if (![writer startWriting]) {
            fail(error, writer.error, "AVFoundation could not start writing the export.");
            return NULL;
        }
        [writer startSessionAtSourceTime:kCMTimeZero];
        if (chapters != nil) {
            int32_t from = settings->from, to = settings->to;
            CMFormatDescriptionRef format = text_format();
            for (NSUInteger i = 0; i < marks.count; i++) {
                int32_t at = MAX([marks[i][0] intValue], from) - from;
                int32_t next = (i + 1 < marks.count ? [marks[i + 1][0] intValue] : to) - from;
                if (next <= at) continue;
                CMSampleBufferRef sample = text_sample(format, marks[i][1], CMTimeMake(at, assembly.fps),
                                                       CMTimeMake(next - at, assembly.fps));
                for (int waited = 0; !chapters.readyForMoreMediaData; waited++) {
                    if (writer.status != AVAssetWriterStatusWriting || waited > 5000) {
                        if (sample != NULL) CFRelease(sample);
                        if (format != NULL) CFRelease(format);
                        [writer cancelWriting];
                        fail(error, writer.error, "The export's chapter track did not take its chapters.");
                        return NULL;
                    }
                    usleep(1000);
                }
                if (sample != NULL) {
                    [chapters appendSampleBuffer:sample];
                    CFRelease(sample);
                }
            }
            if (format != NULL) CFRelease(format);
            [chapters markAsFinished];
        }
        SpliceAVWriter *out = [SpliceAVWriter new];
        out.reader = reader;
        out.writer = writer;
        out.video = video;
        out.adaptor = adaptor;
        out.audio = audio;
        out.url = url;
        out.fps = fps;
        out.surfaces = [NSMutableDictionary dictionary];
        out.audioDone = dispatch_semaphore_create(0);
        out.queue = dispatch_queue_create("splice.export.audio", DISPATCH_QUEUE_SERIAL);
        if (audio != nil) {
            AVAssetReaderAudioMixOutput *source = reader.audio;
            dispatch_semaphore_t done = out.audioDone;
            CMTime shift = CMTimeMake(settings->from, assembly.fps);
            [audio requestMediaDataWhenReadyOnQueue:out.queue usingBlock:^{
                while (audio.readyForMoreMediaData) {
                    CMSampleBufferRef sample = [source copyNextSampleBuffer];
                    CMSampleBufferRef shifted = NULL;
                    if (sample != NULL && CMTimeCompare(shift, kCMTimeZero) != 0) {
                        CMSampleTimingInfo timing;
                        CMSampleBufferGetSampleTimingInfo(sample, 0, &timing);
                        timing.presentationTimeStamp = CMTimeSubtract(timing.presentationTimeStamp, shift);
                        CMSampleBufferCreateCopyWithNewTiming(NULL, sample, 1, &timing, &shifted);
                    }
                    CMSampleBufferRef append = shifted != NULL ? shifted : sample;
                    if (append == NULL || ![audio appendSampleBuffer:append]) {
                        if (sample != NULL) CFRelease(sample);
                        if (shifted != NULL) CFRelease(shifted);
                        [audio markAsFinished];
                        dispatch_semaphore_signal(done);
                        return;
                    }
                    CFRelease(sample);
                    if (shifted != NULL) CFRelease(shifted);
                }
            }];
        } else {
            dispatch_semaphore_signal(out.audioDone);
        }
        return (__bridge_retained void *)out;
    }
}

void *splice_av_writer_target(void *raw, void *device_raw, char **error) {
    @autoreleasepool {
        SpliceAVWriter *writer = (__bridge SpliceAVWriter *)raw;
        id<MTLDevice> device = (__bridge id<MTLDevice>)device_raw;
        if (writer.pending != NULL) {
            CVPixelBufferRelease(writer.pending);
            writer.pending = NULL;
        }
        CVPixelBufferRef pixels = NULL;
        if (writer.adaptor.pixelBufferPool == NULL ||
            CVPixelBufferPoolCreatePixelBuffer(NULL, writer.adaptor.pixelBufferPool, &pixels) != kCVReturnSuccess) {
            splice_set_error(error, "The export ran out of frame buffers.");
            return NULL;
        }
        writer.pending = pixels;
        IOSurfaceRef surface = CVPixelBufferGetIOSurface(pixels);
        NSNumber *key = @(IOSurfaceGetID(surface));
        id<MTLTexture> texture = writer.surfaces[key];
        if (texture == nil) {
            if (writer.surfaces.count >= 64) [writer.surfaces removeAllObjects];
            MTLTextureDescriptor *descriptor = [MTLTextureDescriptor
                texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA8Unorm width:CVPixelBufferGetWidth(pixels)
                                            height:CVPixelBufferGetHeight(pixels) mipmapped:NO];
            descriptor.usage = MTLTextureUsageShaderRead | MTLTextureUsageShaderWrite;
            descriptor.storageMode = MTLStorageModeShared;
            texture = [device newTextureWithDescriptor:descriptor iosurface:surface plane:0];
            if (texture == nil) {
                splice_set_error(error, "Metal could not wrap an export frame.");
                return NULL;
            }
            writer.surfaces[key] = texture;
        }
        return (__bridge void *)texture;
    }
}

bool splice_av_writer_append(void *raw, int32_t frame, char **error) {
    @autoreleasepool {
        SpliceAVWriter *writer = (__bridge SpliceAVWriter *)raw;
        if (writer.pending == NULL) {
            splice_set_error(error, "No export frame was drawn.");
            return false;
        }
        for (int waited = 0; !writer.video.readyForMoreMediaData; waited++) {
            if (writer.writer.status != AVAssetWriterStatusWriting || waited > 10000) {
                fail(error, writer.writer.error, "The encoder stopped taking frames.");
                return false;
            }
            usleep(1000);
        }
        CMTime at = CMTimeMake(frame, writer.fps);
        if (![writer.adaptor appendPixelBuffer:writer.pending withPresentationTime:at]) {
            fail(error, writer.writer.error, "The encoder rejected a frame.");
            return false;
        }
        CVPixelBufferRelease(writer.pending);
        writer.pending = NULL;
        return true;
    }
}

int32_t splice_av_writer_finish(void *raw, bool cancel, char **error) {
    @autoreleasepool {
        SpliceAVWriter *writer = (__bridge SpliceAVWriter *)raw;
        if (cancel) {
            [writer.reader.reader cancelReading];
            [writer.writer cancelWriting];
            [[NSFileManager defaultManager] removeItemAtURL:writer.url error:NULL];
            return 0;
        }
        [writer.video markAsFinished];
        if (writer.video == nil) [writer.reader.reader cancelReading];
        if (dispatch_semaphore_wait(writer.audioDone, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC)) != 0) {
            [writer.writer cancelWriting];
            splice_set_error(error, "The export's audio did not finish.");
            return -1;
        }
        dispatch_semaphore_t finished = dispatch_semaphore_create(0);
        [writer.writer finishWritingWithCompletionHandler:^{ dispatch_semaphore_signal(finished); }];
        dispatch_semaphore_wait(finished, DISPATCH_TIME_FOREVER);
        if (writer.writer.status != AVAssetWriterStatusCompleted) {
            fail(error, writer.writer.error, "AVFoundation could not finish the export.");
            return -1;
        }
        return 1;
    }
}

void splice_av_writer_release(void *raw) {
    @autoreleasepool {
        SpliceAVWriter *writer = (__bridge_transfer SpliceAVWriter *)raw;
        if (writer.writer.status == AVAssetWriterStatusWriting) [writer.writer cancelWriting];
    }
}

#pragma clang diagnostic pop
