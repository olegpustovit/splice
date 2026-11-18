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
