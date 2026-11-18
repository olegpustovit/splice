#include "platform.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreText/CoreText.h>
#import <CoreVideo/CoreVideo.h>
#import <ImageIO/ImageIO.h>
#import <QuartzCore/QuartzCore.h>

#include <dispatch/dispatch.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

enum { SourceVideo, SourceAudio, SourceStill, SourceColor };

@interface SpliceAVTrack : NSObject
@property(nonatomic, strong) AVMutableCompositionTrack *track;
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
@property(nonatomic, strong) NSMutableArray<AVVideoCompositionInstruction *> *instructions;
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

static CGAffineTransform placement(SpliceAVTrack *source, CGSize canvas, const SpliceAVLayer *layer,
                                   double scale, CGRect *raw_crop) {
    CGRect oriented = CGRectApplyAffineTransform(CGRectMake(0, 0, source.size.width, source.size.height),
                                                 source.orientation);
    CGAffineTransform origin = CGAffineTransformConcat(source.orientation,
        CGAffineTransformMakeTranslation(-oriented.origin.x, -oriented.origin.y));
    CGRect crop = CGRectMake(layer->crop[0] * oriented.size.width, layer->crop[1] * oriented.size.height,
                             layer->crop[2] * oriented.size.width, layer->crop[3] * oriented.size.height);
    CGRect dest = rect(layer->destination);
    if (layer->center_crop != 0) {
        double ratio = dest.size.width / dest.size.height;
        double width = fmin(crop.size.width, crop.size.height * ratio);
        double height = fmin(crop.size.height, crop.size.width / ratio);
        crop = CGRectInset(crop, (crop.size.width - width) / 2, (crop.size.height - height) / 2);
    }
    *raw_crop = CGRectApplyAffineTransform(crop, CGAffineTransformInvert(origin));
    dest.origin.x = canvas.width / 2 + (dest.origin.x - canvas.width / 2) * scale;
    dest.origin.y = canvas.height / 2 + (dest.origin.y - canvas.height / 2) * scale;
    CGAffineTransform location = CGAffineTransformConcat(origin, CGAffineTransformMakeTranslation(-crop.origin.x, -crop.origin.y));
    location = CGAffineTransformConcat(location, CGAffineTransformMakeScale(dest.size.width * scale / crop.size.width,
                                                                            dest.size.height * scale / crop.size.height));
    return CGAffineTransformConcat(location, CGAffineTransformMakeTranslation(dest.origin.x, dest.origin.y));
}

static SpliceAVTrack *track_at(SpliceAVAssembly *assembly, int32_t index) {
    return index >= 0 && (NSUInteger)index < assembly.tracks.count ? assembly.tracks[index] : nil;
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
        NSMutableArray *ordered = [NSMutableArray arrayWithCapacity:layer_count];
        // AVFoundation expects the frontmost layer first.
        for (size_t index = layer_count; index > 0; index--) {
            const SpliceAVLayer *layer = &layers[index - 1];
            SpliceAVTrack *source = track_at(assembly, layer->track);
            if (source == nil || source.mix != nil || !valid_rect(layer->crop) || !valid_rect(layer->destination) ||
                !isfinite(layer->scale[0]) || !isfinite(layer->scale[1]) ||
                !(layer->opacity[0] >= 0 && layer->opacity[0] <= 1 &&
                  layer->opacity[1] >= 0 && layer->opacity[1] <= 1)) {
                splice_set_error(error, "Invalid picture layer in a composition instruction.");
                return false;
            }
            AVMutableVideoCompositionLayerInstruction *instruction =
                [AVMutableVideoCompositionLayerInstruction videoCompositionLayerInstructionWithAssetTrack:source.track];
            CGRect crop;
            CGAffineTransform a = placement(source, assembly.size, layer, layer->scale[0], &crop);
            CGAffineTransform b = placement(source, assembly.size, layer, layer->scale[1], &crop);
            [instruction setCropRectangle:crop atTime:start];
            [instruction setTransformRampFromStartTransform:a toEndTransform:b timeRange:interval];
            [instruction setOpacityRampFromStartOpacity:(float)layer->opacity[0]
                                           toEndOpacity:(float)layer->opacity[1] timeRange:interval];
            [ordered addObject:instruction];
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
        AVMutableVideoCompositionInstruction *instruction = [AVMutableVideoCompositionInstruction videoCompositionInstruction];
        instruction.timeRange = interval;
        instruction.layerInstructions = ordered;
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
        CALayer *layer = [CALayer layer];
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
        double first = (double)from / assembly.end;
        double last = ((double)to - 0.5) / assembly.end;
        NSMutableArray *times = [NSMutableArray array];
        NSMutableArray *values = [NSMutableArray array];
        if (from > 0) {
            [times addObjectsFromArray:@[@0, @(((double)from - 0.5) / assembly.end)]];
            [values addObjectsFromArray:@[@0, @0]];
        }
        double fraction = ((double)(to - from) - 0.5) / (to - from);
        [times addObjectsFromArray:@[@(first), @(last), @((double)to / assembly.end)]];
        [values addObjectsFromArray:@[@(spec->opacity[0]),
                                      @(spec->opacity[0] + (spec->opacity[1] - spec->opacity[0]) * fraction),
                                      @0]];
        if (to < assembly.end) {
            [times addObject:@1];
            [values addObject:@0];
        }
        CAKeyframeAnimation *show = [CAKeyframeAnimation animationWithKeyPath:@"opacity"];
        show.values = values;
        show.keyTimes = times;
        show.beginTime = AVCoreAnimationBeginTimeAtZero;
        show.duration = (double)assembly.end / assembly.fps;
        show.fillMode = kCAFillModeBoth;
        show.removedOnCompletion = NO;
        [layer addAnimation:show forKey:@"visibility"];
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
        video.renderSize = assembly.size;
        video.frameDuration = time_for(assembly, 1);
        video.instructions = assembly.instructions;
        assembly.video = video;
        // ponytail: export uses Core Animation text and the viewer draws evaluator text in PXUI.
        // Use the M3 compositor for both if a visible mismatch is reported in friction.
        if (assembly.text != nil) {
            CALayer *root = [CALayer layer];
            root.frame = assembly.text.frame;
            CALayer *picture = [CALayer layer];
            picture.frame = root.bounds;
            [root addSublayer:picture];
            [root addSublayer:assembly.text];
            assembly.offline = [video mutableCopy];
            assembly.offline.animationTool = [AVVideoCompositionCoreAnimationTool
                 videoCompositionCoreAnimationToolWithPostProcessingAsVideoLayer:picture inLayer:root];
            [CATransaction flush];
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

bool splice_av_read_frame(void *raw, int32_t frame, uint8_t *rgba, size_t length, char **error) {
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
        CGContextRelease(context);
        CGImageRelease(image);
        return true;
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
