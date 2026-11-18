#include "recording_internal.h"

#import <CoreVideo/CoreVideo.h>

#include <math.h>
#include <string.h>

static CMSampleBufferRef fixture_video(CVPixelBufferRef pixels, CMVideoFormatDescriptionRef format,
                                      CMTime pts, SCFrameStatus status) {
    CMSampleTimingInfo timing = {CMTimeMake(1, 60), pts, kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    if (CMSampleBufferCreateReadyWithImageBuffer(NULL, pixels, format, &timing, &sample) != noErr) {
        return NULL;
    }
    NSArray *attachments = (__bridge NSArray *)CMSampleBufferGetSampleAttachmentsArray(sample, true);
    [attachments.firstObject setObject:@(status) forKey:SCStreamFrameInfoStatus];
    return sample;
}

static CMSampleBufferRef fixture_audio(CMAudioFormatDescriptionRef format, CMTime pts) {
    const size_t frame_size = 4;
    const CMItemCount frames = 800;
    const size_t bytes = frame_size * (size_t)frames;
    CMBlockBufferRef block = NULL;
    if (CMBlockBufferCreateWithMemoryBlock(NULL, NULL, bytes, NULL, NULL, 0, bytes,
                                          kCMBlockBufferAssureMemoryNowFlag, &block) != noErr) {
        return NULL;
    }
    CMSampleTimingInfo timing = {CMTimeMake(1, 48000), pts, kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    if (CMBlockBufferFillDataBytes(0, block, 0, bytes) == noErr) {
        CMSampleBufferCreateReady(NULL, block, format, frames, 1, &timing, 1, &frame_size, &sample);
    }
    CFRelease(block);
    return sample;
}

// The fixture reads the actual QuickTime timestamps with the macOS 15 reader API.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static bool camera_clock_matches(NSURL *directory, char **error) {
    AVAsset *asset = [AVURLAsset URLAssetWithURL:[directory URLByAppendingPathComponent:@"camera.mov"] options:nil];
    AVAssetTrack *track = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
    NSArray<AVAssetTrackSegment *> *segments = track.segments;
    if (segments.count != 2 || !segments[0].empty || segments[1].empty ||
        fabs(CMTimeGetSeconds(segments[1].timeMapping.target.start) - 2.0 / 60.0) > 0.0001) {
        splice_set_error(error, "Camera fixture lost its two-frame edit-list offset.");
        return false;
    }
    NSError *reason = nil;
    AVAssetReader *reader = [[AVAssetReader alloc] initWithAsset:asset error:&reason];
    if (reader == nil || track == nil) {
        splice_set_error(error, reason.localizedDescription.UTF8String ?: "Camera fixture has no video track.");
        return false;
    }
    AVAssetReaderTrackOutput *output = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:track outputSettings:nil];
    if (![reader canAddOutput:output]) {
        splice_set_error(error, "Cannot read the camera fixture's video track.");
        return false;
    }
    [reader addOutput:output];
    if (![reader startReading]) {
        splice_set_error(error, reader.error.localizedDescription.UTF8String ?: "Could not read camera fixture.");
        return false;
    }
    // A passthrough reader returns track media time. The edit list above supplies movie time.
    // The third frame follows a simulated one-second pause.
    int frames = 0;
    double first_pts = 0.0;
    CMSampleBufferRef sample;
    while (frames < 3 && (sample = [output copyNextSampleBuffer]) != NULL) {
        double pts = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample));
        double duration = CMTimeGetSeconds(CMSampleBufferGetDuration(sample));
        CFRelease(sample);
        if (!isfinite(pts) || !(duration > 0.0)) continue;
        if (frames == 0) first_pts = pts;
        if (fabs(pts - first_pts - frames / 60.0) > 0.0001) break;
        frames++;
    }
    if (frames == 3) return true;
    splice_set_error(error, "Camera fixture contains a timestamp gap after resume.");
    return false;
}
#pragma clang diagnostic pop

bool splice_recording_fixture(const char *directory, SpliceRecordingStats *stats, char **error) {
    @autoreleasepool {
        NSError *failure = nil;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:directory isDirectory:YES relativeToURL:nil];
        SpliceRecording *recording = [[SpliceRecording alloc] initWithDirectory:url width:96 height:64
                                                                  microphone:YES audio:YES camera:YES error:&failure];
        if (recording == nil) {
            splice_set_error(error, failure.localizedDescription.UTF8String);
            return false;
        }
        CVPixelBufferRef pixels = NULL;
        CMVideoFormatDescriptionRef video = NULL;
        CMAudioFormatDescriptionRef audio = NULL;
        AudioStreamBasicDescription pcm = {
            .mSampleRate = 48000, .mFormatID = kAudioFormatLinearPCM,
            .mFormatFlags = kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
            .mBytesPerPacket = 4, .mFramesPerPacket = 1, .mBytesPerFrame = 4,
            .mChannelsPerFrame = 2, .mBitsPerChannel = 16,
        };
        NSDictionary *attributes = @{(__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{}};
        bool ready = CVPixelBufferCreate(NULL, 96, 64, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                                         (__bridge CFDictionaryRef)attributes, &pixels) == kCVReturnSuccess &&
            CMVideoFormatDescriptionCreateForImageBuffer(NULL, pixels, &video) == noErr &&
            CMAudioFormatDescriptionCreate(NULL, &pcm, 0, NULL, 0, NULL, NULL, &audio) == noErr &&
            CVPixelBufferLockBaseAddress(pixels, 0) == kCVReturnSuccess;
        if (ready) {
            for (size_t plane = 0; plane < 2; plane++) {
                memset(CVPixelBufferGetBaseAddressOfPlane(pixels, plane), plane == 0 ? 64 : 128,
                       CVPixelBufferGetBytesPerRowOfPlane(pixels, plane) * CVPixelBufferGetHeightOfPlane(pixels, plane));
            }
            CVPixelBufferUnlockBaseAddress(pixels, 0);
            __block bool samples_ok = true;
            dispatch_sync(recording.queue, ^{
                CMSampleBufferRef idle = fixture_video(pixels, video, CMTimeMake(599, 60), SCFrameStatusIdle);
                if (idle == NULL) {
                    samples_ok = false;
                    return;
                }
                [recording append:idle type:SCStreamOutputTypeScreen];
                [recording appendCamera:idle atHostTime:CMTimeMake(599, 60)];
                CFRelease(idle);
                for (int32_t frame = 0; frame < 8; frame++) {
                    if (frame == 4) {
                        [recording setPaused:YES at:CMTimeMake(604, 60)];
                        CMSampleBufferRef paused = fixture_video(pixels, video, CMTimeMake(630, 60), SCFrameStatusComplete);
                        if (paused == NULL) {
                            samples_ok = false;
                            return;
                        }
                        [recording append:paused type:SCStreamOutputTypeScreen];
                        [recording appendCamera:paused atHostTime:CMTimeMake(630, 60)];
                        [recording setPaused:NO at:CMTimeMake(664, 60)];
                        [recording append:paused type:SCStreamOutputTypeScreen];
                        [recording appendCamera:paused atHostTime:CMTimeMake(630, 60)];
                        CFRelease(paused);
                    }
                    CMTime pts = CMTimeMake(600 + frame + (frame >= 4 ? 60 : 0), 60);
                    CMSampleBufferRef screen = fixture_video(pixels, video, pts, SCFrameStatusComplete);
                    // Camera timestamps start on a different clock and begin two frames later.
                    CMSampleBufferRef camera = frame < 2 ? NULL : fixture_video(
                        pixels, video, CMTimeAdd(pts, CMTimeMake(10000, 60)), SCFrameStatusComplete);
                    CMSampleBufferRef sound = fixture_audio(audio, pts);
                    samples_ok = screen != NULL && sound != NULL && (frame < 2 || camera != NULL);
                    if (samples_ok) {
                        [recording append:screen type:SCStreamOutputTypeScreen];
                        [recording append:sound type:SCStreamOutputTypeMicrophone];
                        [recording append:sound type:SCStreamOutputTypeAudio];
                        if (camera != NULL) [recording appendCamera:camera atHostTime:pts];
                    }
                    if (screen != NULL) CFRelease(screen);
                    if (sound != NULL) CFRelease(sound);
                    if (camera != NULL) CFRelease(camera);
                    if (!samples_ok) return;
                    [NSThread sleepForTimeInterval:0.005];
                }
            });
            ready = samples_ok;
        }
        if (video != NULL) CFRelease(video);
        if (audio != NULL) CFRelease(audio);
        if (pixels != NULL) CFRelease(pixels);
        if (!ready) {
            [recording cancel];
            splice_set_error(error, "Could not construct synthetic capture samples.");
            return false;
        }
        CMTime end = CMTimeMake(720, 60);
        failure = [recording finishAt:end];
        if (failure != nil) {
            splice_set_error(error, failure.localizedDescription.UTF8String);
            return false;
        }
        dispatch_sync(recording.queue, ^{ *stats = [recording statsAt:end]; });
        return camera_clock_matches(url, error);
    }
}
