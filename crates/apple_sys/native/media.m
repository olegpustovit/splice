#include "platform.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreMedia/CoreMedia.h>
#import <Foundation/Foundation.h>

#include <dispatch/dispatch.h>
#include <errno.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

// Objective-C's asynchronous key loader is available on the macOS 15 deployment target.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static bool load_keys(id<AVAsynchronousKeyValueLoading> object, NSArray<NSString *> *keys,
                      dispatch_time_t deadline, char **error) {
    dispatch_semaphore_t finished = dispatch_semaphore_create(0);
    [object loadValuesAsynchronouslyForKeys:keys completionHandler:^{
        dispatch_semaphore_signal(finished);
    }];
    if (dispatch_semaphore_wait(finished, deadline) != 0) {
        splice_set_error(error, "Timed out loading AVFoundation metadata.");
        return false;
    }
    for (NSString *key in keys) {
        NSError *failure = nil;
        if ([object statusOfValueForKey:key error:&failure] != AVKeyValueStatusLoaded) {
            const char *description = failure.localizedDescription.UTF8String;
            splice_set_error(error, description != NULL ? description : "AVFoundation metadata is unavailable.");
            return false;
        }
    }
    return true;
}

static bool probe_video(AVAssetTrack *track, SpliceMediaProbe *out, dispatch_time_t deadline, char **error) {
    if (!load_keys(track, @[@"naturalSize", @"preferredTransform", @"nominalFrameRate", @"formatDescriptions"],
                   deadline, error)) {
        return false;
    }
    CGSize size = track.naturalSize;
    CGRect bounds = CGRectApplyAffineTransform(CGRectMake(0, 0, size.width, size.height), track.preferredTransform);
    double width = round(CGRectGetWidth(bounds));
    double height = round(CGRectGetHeight(bounds));
    if (!isfinite(width) || !isfinite(height) || width < 1 || height < 1 ||
        width > UINT32_MAX || height > UINT32_MAX) {
        splice_set_error(error, "The video track has invalid display dimensions.");
        return false;
    }
    out->width = (uint32_t)width;
    out->height = (uint32_t)height;
    double fps = track.nominalFrameRate;
    out->nominal_fps = isfinite(fps) && fps > 0 ? fps : 0;
    CMFormatDescriptionRef format = (__bridge_retained CMFormatDescriptionRef)track.formatDescriptions.firstObject;
    if (format != NULL) {
        out->video_codec = CMFormatDescriptionGetMediaSubType(format);
        CFRelease(format);
    }
    return true;
}

static bool probe_audio(NSArray<AVAssetTrack *> *tracks, SpliceMediaProbe *out,
                        dispatch_time_t deadline, char **error) {
    if (tracks.count == 0) {
        return true;
    }
    out->audio = calloc(tracks.count, sizeof(SpliceAudioTrack));
    if (out->audio == NULL) {
        splice_set_error(error, "Could not allocate the audio track metadata.");
        return false;
    }
    out->audio_count = tracks.count;
    for (NSUInteger index = 0; index < tracks.count; index++) {
        AVAssetTrack *track = tracks[index];
        if (!load_keys(track, @[@"formatDescriptions"], deadline, error)) {
            return false;
        }
        SpliceAudioTrack *destination = &out->audio[index];
        destination->track_id = track.trackID;
        CMFormatDescriptionRef format = (__bridge_retained CMFormatDescriptionRef)track.formatDescriptions.firstObject;
        if (format != NULL) {
            destination->codec = CMFormatDescriptionGetMediaSubType(format);
            const AudioStreamBasicDescription *audio = CMAudioFormatDescriptionGetStreamBasicDescription(format);
            if (audio != NULL) {
                destination->channels = audio->mChannelsPerFrame;
            }
            CFRelease(format);
        }
    }
    return true;
}

bool splice_media_probe(const char *path, SpliceMediaProbe *out, char **error) {
    @autoreleasepool {
        *out = (SpliceMediaProbe){0};
        struct stat info;
        if (stat(path, &info) != 0) {
            splice_set_error(error, strerror(errno));
            return false;
        }
        if (!S_ISREG(info.st_mode)) {
            splice_set_error(error, "Media probing requires a regular local file.");
            return false;
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        if (url == nil) {
            splice_set_error(error, "Could not open the local media URL.");
            return false;
        }
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:@{AVURLAssetPreferPreciseDurationAndTimingKey: @YES}];
        dispatch_time_t deadline = dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC);
        if (!load_keys(asset, @[@"duration", @"tracks"], deadline, error)) {
            return false;
        }

        double duration = CMTimeGetSeconds(asset.duration);
        if (!isfinite(duration) || duration < 0) {
            splice_set_error(error, "AVFoundation did not provide a finite media duration.");
            return false;
        }
        out->duration_seconds = duration;
        NSArray<AVAssetTrack *> *video = [asset tracksWithMediaType:AVMediaTypeVideo];
        NSArray<AVAssetTrack *> *audio = [asset tracksWithMediaType:AVMediaTypeAudio];
        if (video.count == 0 && audio.count == 0) {
            splice_set_error(error, "The file contains no supported audio or video tracks.");
            return false;
        }
        if (video.count > UINT32_MAX) {
            splice_set_error(error, "Too many video tracks.");
            return false;
        }
        out->video_tracks = (uint32_t)video.count;
        if (video.count > 0 && !probe_video(video.firstObject, out, deadline, error)) {
            return false;
        }
        return probe_audio(audio, out, deadline, error);
    }
}

#pragma clang diagnostic pop

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
// Integrated loudness (ITU-R BS.1770-4) of the file's audio, mixed to stereo at 48 kHz: K-weighting,
// 400 ms blocks every 100 ms, absolute (-70 LUFS) and relative (-10 LU) gates. NaN without audio.
double splice_media_loudness(const char *path, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
        NSArray<AVAssetTrack *> *tracks = [asset tracksWithMediaType:AVMediaTypeAudio];
        if (tracks.count == 0) return NAN;
        NSError *failure = nil;
        AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&failure];
        AVAssetReaderAudioMixOutput *output = [AVAssetReaderAudioMixOutput
            assetReaderAudioMixOutputWithAudioTracks:tracks audioSettings:@{
                AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @48000, AVNumberOfChannelsKey: @2,
                AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES, AVLinearPCMIsNonInterleaved: @NO,
                AVLinearPCMIsBigEndianKey: @NO}];
        if (reader == nil || ![reader canAddOutput:output]) {
            if (error != NULL) *error = strdup(failure.localizedDescription.UTF8String ?: "Could not read the audio.");
            return NAN;
        }
        [reader addOutput:output];
        if (![reader startReading]) {
            if (error != NULL) *error = strdup("Could not start reading the audio.");
            return NAN;
        }
        // K-weighting biquads at 48 kHz: a high shelf, then a high pass (BS.1770-4 table 1, 2).
        const double b1[3] = {1.53512485958697, -2.69169618940638, 1.19839281085285};
        const double a1[3] = {1.0, -1.69065929318241, 0.73248077421585};
        const double b2[3] = {1.0, -2.0, 1.0};
        const double a2[3] = {1.0, -1.99004745483398, 0.99007225036621};
        double state[2][4] = {{0}};  // per channel: stage 1 z1, z2, stage 2 z1, z2
        const size_t hop = 4800;     // 100 ms
        double sub[4] = {0};         // the last four 100 ms sums of squares
        size_t filled = 0, count = 0, subs = 0;
        double current = 0;
        NSMutableData *blocks = [NSMutableData data];  // mean square per 400 ms block
        CMSampleBufferRef sample;
        while ((sample = [output copyNextSampleBuffer]) != NULL) {
            CMBlockBufferRef block = CMSampleBufferGetDataBuffer(sample);
            size_t length = 0;
            char *data = NULL;
            if (block != NULL && CMBlockBufferGetDataPointer(block, 0, NULL, &length, &data) == kCMBlockBufferNoErr) {
                const float *pcm = (const float *)data;
                for (size_t frame = 0; frame + 1 < length / sizeof(float); frame += 2) {
                    double energy = 0;
                    for (int c = 0; c < 2; c++) {
                        double *z = state[c];
                        double x = pcm[frame + c];
                        double y = b1[0] * x + z[0];
                        z[0] = b1[1] * x - a1[1] * y + z[1];
                        z[1] = b1[2] * x - a1[2] * y;
                        double w = b2[0] * y + z[2];
                        z[2] = b2[1] * y - a2[1] * w + z[3];
                        z[3] = b2[2] * y - a2[2] * w;
                        energy += w * w;
                    }
                    current += energy;
                    if (++count == hop) {
                        sub[filled % 4] = current;
                        filled++, subs++, count = 0, current = 0;
                        if (subs >= 4) {
                            double z = (sub[0] + sub[1] + sub[2] + sub[3]) / (4.0 * hop);
                            [blocks appendBytes:&z length:sizeof z];
                        }
                    }
                }
            }
            CFRelease(sample);
        }
        if (reader.status == AVAssetReaderStatusFailed) {
            if (error != NULL) *error = strdup(reader.error.localizedDescription.UTF8String ?: "Reading the audio failed.");
            return NAN;
        }
        const double *z = blocks.bytes;
        size_t n = blocks.length / sizeof(double);
        double sum = 0;
        size_t kept = 0;
        for (size_t i = 0; i < n; i++) {
            if (-0.691 + 10 * log10(z[i]) > -70) sum += z[i], kept++;
        }
        if (kept == 0) return -70;  // silence
        double relative = -0.691 + 10 * log10(sum / kept) - 10;
        double gated = 0;
        size_t final = 0;
        for (size_t i = 0; i < n; i++) {
            double l = -0.691 + 10 * log10(z[i]);
            if (l > -70 && l > relative) gated += z[i], final++;
        }
        return final == 0 ? -70 : -0.691 + 10 * log10(gated / final);
    }
}
#pragma clang diagnostic pop

char *splice_media_bookmark(const char *path, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        NSError *failure = nil;
        NSData *data = [url bookmarkDataWithOptions:0 includingResourceValuesForKeys:nil
                                      relativeToURL:nil error:&failure];
        if (data.length == 0) {
            const char *description = failure.localizedDescription.UTF8String;
            splice_set_error(error, description != NULL ? description : "Could not create a file bookmark.");
            return NULL;
        }
        const char *text = [data base64EncodedStringWithOptions:0].UTF8String;
        char *encoded = text != NULL ? strdup(text) : NULL;
        if (encoded == NULL) {
            splice_set_error(error, "Could not allocate the file bookmark.");
        }
        return encoded;
    }
}

void splice_media_dispose(SpliceMediaProbe *probe) {
    free(probe->audio);
    *probe = (SpliceMediaProbe){0};
}

// Free bytes on the volume holding `path` (export disk checks, PLAN.md T21.5).
bool splice_free_space(const char *path, uint64_t *bytes) {
    @autoreleasepool {
        NSDictionary *attributes = [[NSFileManager defaultManager]
            attributesOfFileSystemForPath:[NSString stringWithUTF8String:path] error:nil];
        NSNumber *free = attributes[NSFileSystemFreeSize];
        if (free == nil) return false;
        *bytes = free.unsignedLongLongValue;
        return true;
    }
}

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
// The video frame at asset time as RGBA, with an optional display transform.
uint8_t *splice_media_frame_at(const char *path, double seconds, bool oriented, uint32_t *width, uint32_t *height, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVAssetImageGenerator *generator = [AVAssetImageGenerator assetImageGeneratorWithAsset:[AVURLAsset assetWithURL:url]];
        generator.appliesPreferredTrackTransform = oriented;
        generator.requestedTimeToleranceBefore = kCMTimeZero;
        generator.requestedTimeToleranceAfter = kCMTimeZero;
        NSError *failure = nil;
        CGImageRef image = [generator copyCGImageAtTime:CMTimeMakeWithSeconds(seconds, 600000) actualTime:NULL error:&failure];
        if (image == NULL) {
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "No frame.");
            return NULL;
        }
        size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
        if (w == 0 || h == 0 || w > UINT32_MAX || h > UINT32_MAX || w > PTRDIFF_MAX / h / 4) {
            CGImageRelease(image);
            splice_set_error(error, "Source frame dimensions are invalid.");
            return NULL;
        }
        uint8_t *pixels = calloc(w * h, 4);
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = pixels != NULL && space != NULL
            ? CGBitmapContextCreate(pixels, w, h, 8, w * 4, space,
                                    kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast)
            : NULL;
        if (space != NULL) CGColorSpaceRelease(space);
        if (context == NULL) {
            free(pixels);
            CGImageRelease(image);
            splice_set_error(error, "Could not allocate source frame pixels.");
            return NULL;
        }
        CGContextDrawImage(context, CGRectMake(0, 0, w, h), image);
        CGContextRelease(context);
        CGImageRelease(image);
        *width = (uint32_t)w;
        *height = (uint32_t)h;
        return pixels;
    }
}

// Chapter titles and start seconds ("start\ttitle" lines) as AVAsset reads them.
char *splice_media_chapters(const char *path, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVAsset *asset = [AVURLAsset assetWithURL:url];
        NSMutableArray<NSString *> *languages = [NSMutableArray arrayWithObject:@"en"];
        for (NSLocale *locale in asset.availableChapterLocales) [languages addObject:locale.localeIdentifier];
        NSArray<AVTimedMetadataGroup *> *groups =
            [asset chapterMetadataGroupsBestMatchingPreferredLanguages:languages];
        NSMutableString *out = [NSMutableString string];
        for (AVTimedMetadataGroup *group in groups) {
            NSString *title = @"";
            for (AVMetadataItem *item in group.items) {
                if ([item.commonKey isEqualToString:AVMetadataCommonKeyTitle]) title = item.stringValue ?: @"";
            }
            [out appendFormat:@"%.3f\t%@\n", CMTimeGetSeconds(group.timeRange.start), title];
        }
        (void)error;
        return strdup(out.UTF8String);
    }
}
#pragma clang diagnostic pop
