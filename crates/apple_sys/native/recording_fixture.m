#include "recording_internal.h"

#import <CoreVideo/CoreVideo.h>

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

bool splice_recording_fixture(const char *directory, SpliceRecordingStats *stats, char **error) {
    @autoreleasepool {
        NSError *failure = nil;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:directory isDirectory:YES relativeToURL:nil];
        SpliceRecording *recording = [[SpliceRecording alloc] initWithDirectory:url width:96 height:64
                                                                  microphone:YES audio:YES error:&failure];
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
                        [recording setPaused:NO at:CMTimeMake(664, 60)];
                        [recording append:paused type:SCStreamOutputTypeScreen];
                        CFRelease(paused);
                    }
                    CMTime pts = CMTimeMake(600 + frame + (frame >= 4 ? 60 : 0), 60);
                    CMSampleBufferRef screen = fixture_video(pixels, video, pts, SCFrameStatusComplete);
                    CMSampleBufferRef sound = fixture_audio(audio, pts);
                    samples_ok = screen != NULL && sound != NULL;
                    if (samples_ok) {
                        [recording append:screen type:SCStreamOutputTypeScreen];
                        [recording append:sound type:SCStreamOutputTypeMicrophone];
                        [recording append:sound type:SCStreamOutputTypeAudio];
                    }
                    if (screen != NULL) CFRelease(screen);
                    if (sound != NULL) CFRelease(sound);
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
        return true;
    }
}
