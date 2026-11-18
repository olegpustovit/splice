#include "recording_internal.h"

#import <CoreVideo/CoreVideo.h>

#include <math.h>
#include <stdlib.h>

static NSError *recording_error(NSString *message) {
    return [NSError errorWithDomain:@"SpliceRecording" code:1
                           userInfo:@{NSLocalizedDescriptionKey: message}];
}

static CMTime host_time(void) {
    return CMClockGetTime(CMClockGetHostTimeClock());
}

static CMTime positive(CMTime time) {
    return CMTimeMaximum(kCMTimeZero, time);
}

static NSDictionary *video_settings(NSInteger width, NSInteger height, NSNumber *frame_rate) {
    return @{
        AVVideoCodecKey: AVVideoCodecTypeHEVC, AVVideoWidthKey: @(width), AVVideoHeightKey: @(height),
        AVVideoCompressionPropertiesKey: @{
            AVVideoAverageBitRateKey: @(MIN(width * height * 4, 60000000)),
            AVVideoExpectedSourceFrameRateKey: frame_rate, AVVideoAllowFrameReorderingKey: @NO,
        },
    };
}

static CMSampleBufferRef retime(CMSampleBufferRef sample, CMTime offset) {
    CMItemCount count = 0;
    if (CMSampleBufferGetSampleTimingInfoArray(sample, 0, NULL, &count) != noErr ||
        count <= 0 || count > 4096) return NULL;
    CMSampleTimingInfo one;
    CMSampleTimingInfo *timings = count == 1 ? &one : calloc((size_t)count, sizeof(*timings));
    if (timings == NULL) return NULL;
    CMSampleBufferRef result = NULL;
    if (CMSampleBufferGetSampleTimingInfoArray(sample, count, timings, NULL) == noErr) {
        for (CMItemCount i = 0; i < count; ++i) {
            timings[i].presentationTimeStamp = CMTimeSubtract(timings[i].presentationTimeStamp, offset);
            if (CMTIME_IS_NUMERIC(timings[i].decodeTimeStamp)) {
                timings[i].decodeTimeStamp = CMTimeSubtract(timings[i].decodeTimeStamp, offset);
            }
        }
        if (CMSampleBufferCreateCopyWithNewTiming(NULL, sample, count, timings, &result) != noErr) {
            result = NULL;
        }
    }
    if (timings != &one) free(timings);
    return result;
}

@interface SpliceRecordingTrack : NSObject
@property(nonatomic, readonly) AVAssetWriter *writer;
@property(nonatomic, readonly) NSUInteger count;
- (instancetype)initWithURL:(NSURL *)url video:(BOOL)video settings:(NSDictionary *)settings error:(NSError **)error;
- (BOOL)append:(CMSampleBufferRef)sample offset:(CMTime)offset;
- (NSError *)sealAt:(CMTime)end group:(dispatch_group_t)group;
- (void)cancel;
@end

@implementation SpliceRecordingTrack {
    AVAssetWriterInput *_input;
    CMTime _lastTime;
    CMSampleBufferRef _lastVideo;
}

// These AVAssetWriterInput methods are the real-time buffer API on macOS 15.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
- (instancetype)initWithURL:(NSURL *)url video:(BOOL)video settings:(NSDictionary *)settings error:(NSError **)error {
    self = [super init];
    if (self == nil) return nil;
    AVMediaType type = video ? AVMediaTypeVideo : AVMediaTypeAudio;
    _writer = [[AVAssetWriter alloc] initWithURL:url fileType:video ? AVFileTypeQuickTimeMovie : AVFileTypeAppleM4A error:error];
    if (_writer == nil) return nil;
    if (![_writer canApplyOutputSettings:settings forMediaType:type]) {
        *error = recording_error(@"The native encoder does not support the capture settings.");
        return nil;
    }
    _input = [AVAssetWriterInput assetWriterInputWithMediaType:type outputSettings:settings];
    _input.expectsMediaDataInRealTime = YES;
    if (![_writer canAddInput:_input]) {
        *error = recording_error(@"The recording writer could not configure its input.");
        return nil;
    }
    [_writer addInput:_input];
    _lastTime = kCMTimeInvalid;
    return self;
}

- (BOOL)append:(CMSampleBufferRef)sample offset:(CMTime)offset {
    CMTime pts = CMTimeSubtract(CMSampleBufferGetPresentationTimeStamp(sample), offset);
    if (!CMTIME_IS_NUMERIC(pts) || CMTimeCompare(pts, kCMTimeZero) < 0 ||
        (CMTIME_IS_NUMERIC(_lastTime) && CMTimeCompare(pts, _lastTime) <= 0)) return NO;
    if (_writer.status == AVAssetWriterStatusUnknown) {
        if (![_writer startWriting]) return NO;
        [_writer startSessionAtSourceTime:kCMTimeZero];
    }
    if (_writer.status != AVAssetWriterStatusWriting || !_input.readyForMoreMediaData) return NO;
    CMSampleBufferRef shifted = retime(sample, offset);
    if (shifted == NULL) return NO;
    BOOL appended = [_input appendSampleBuffer:shifted];
    CFRelease(shifted);
    if (appended) {
        _lastTime = pts;
        _count++;
        if ([_input.mediaType isEqualToString:AVMediaTypeVideo]) {
            CFRetain(sample);
            if (_lastVideo != NULL) CFRelease(_lastVideo);
            _lastVideo = sample;
        }
    }
    return appended;
}

- (NSError *)sealAt:(CMTime)end group:(dispatch_group_t)group {
    if (_writer.status != AVAssetWriterStatusWriting || self.count == 0) {
        [self cancel];
        return _writer.error ?: recording_error(@"A requested capture track delivered no samples.");
    }
    if (_lastVideo != NULL) {
        CMTime tail = CMTimeSubtract(end, CMTimeMake(1, 60));
        if (CMTimeCompare(tail, _lastTime) > 0) {
            // Preserve the last image through Stop, including idle screen intervals.
            for (int i = 0; i < 40 && !_input.readyForMoreMediaData &&
                            _writer.status == AVAssetWriterStatusWriting; ++i) {
                [NSThread sleepForTimeInterval:0.01];
            }
            CMTime offset = CMTimeSubtract(CMSampleBufferGetPresentationTimeStamp(_lastVideo), tail);
            if (![self append:_lastVideo offset:offset]) {
                return _writer.error ?: recording_error(@"Could not hold the final video frame.");
            }
        }
    }
    [_writer endSessionAtSourceTime:end];
    [_input markAsFinished];
    dispatch_group_enter(group);
    [_writer finishWritingWithCompletionHandler:^{ dispatch_group_leave(group); }];
    return nil;
}
#pragma clang diagnostic pop

- (void)cancel {
    if (_writer.status == AVAssetWriterStatusWriting) [_writer cancelWriting];
}

- (void)dealloc {
    if (_lastVideo != NULL) CFRelease(_lastVideo);
}
@end

@implementation SpliceRecording {
    SpliceRecordingTrack *_screen, *_microphone, *_audio, *_camera;
    NSURL *_cameraURL;
    AVCaptureSession *_cameraSession;
    AVCaptureVideoDataOutput *_cameraOutput;
    dispatch_queue_t _cameraControl;
    id _cameraErrorObserver;
    NSError *_failure;
    CMTime _first, _pausedSoFar, _pauseStart, _resumeCutoff, _finalTime;
    uint64_t _frames, _dropped;
    BOOL _stopping;
}

- (instancetype)initWithDirectory:(NSURL *)directory width:(NSInteger)width height:(NSInteger)height
                       microphone:(BOOL)microphone audio:(BOOL)audio camera:(BOOL)camera error:(NSError **)error {
    self = [super init];
    if (self == nil) return nil;
    _queue = dispatch_queue_create("com.olegpustovit.splice.capture", DISPATCH_QUEUE_SERIAL);
    _first = _pauseStart = _resumeCutoff = _finalTime = kCMTimeInvalid;
    _pausedSoFar = kCMTimeZero;
    NSDictionary *sound = @{
        AVFormatIDKey: @(kAudioFormatMPEG4AAC), AVSampleRateKey: @48000,
        AVNumberOfChannelsKey: @2, AVEncoderBitRateKey: @192000,
    };
    _screen = [[SpliceRecordingTrack alloc] initWithURL:[directory URLByAppendingPathComponent:@"screen.mov"]
                                                video:YES settings:video_settings(width, height, @60) error:error];
    if (_screen == nil) return nil;
    if (microphone) {
        _microphone = [[SpliceRecordingTrack alloc] initWithURL:[directory URLByAppendingPathComponent:@"mic.m4a"]
                                                        video:NO settings:sound error:error];
        if (_microphone == nil) return nil;
    }
    if (audio) {
        _audio = [[SpliceRecordingTrack alloc] initWithURL:[directory URLByAppendingPathComponent:@"system.m4a"]
                                                   video:NO settings:sound error:error];
        if (_audio == nil) return nil;
    }
    if (camera) _cameraURL = [directory URLByAppendingPathComponent:@"camera.mov"];
    return self;
}

- (void)cameraFailed:(NSError *)error {
    dispatch_async(_queue, ^{
        if (!self->_stopping && self->_failure == nil) self->_failure = error;
    });
}

- (BOOL)prepareCamera:(NSError **)error {
    AVCaptureDevice *device = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
    if (device == nil) {
        *error = recording_error(@"No default camera is available.");
        return NO;
    }
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:error];
    if (input == nil) return NO;
    AVCaptureSession *session = [AVCaptureSession new];
    AVCaptureVideoDataOutput *output = [AVCaptureVideoDataOutput new];
    output.alwaysDiscardsLateVideoFrames = YES;
    [session beginConfiguration];
    if ([session canSetSessionPreset:AVCaptureSessionPresetHigh]) session.sessionPreset = AVCaptureSessionPresetHigh;
    if (![session canAddInput:input]) {
        [session commitConfiguration];
        *error = recording_error(@"The camera input cannot be added to the capture session.");
        return NO;
    }
    [session addInput:input];
    if (![session canAddOutput:output]) {
        [session commitConfiguration];
        *error = recording_error(@"The camera output cannot be added to the capture session.");
        return NO;
    }
    [session addOutput:output];
    [session commitConfiguration];
    [output setSampleBufferDelegate:self queue:_queue];
    _cameraSession = session;
    _cameraOutput = output;
    _cameraControl = dispatch_queue_create("com.olegpustovit.splice.camera", DISPATCH_QUEUE_SERIAL);
    __weak SpliceRecording *weak = self;
    _cameraErrorObserver = [[NSNotificationCenter defaultCenter]
        addObserverForName:AVCaptureSessionRuntimeErrorNotification object:session queue:nil
        usingBlock:^(NSNotification *notification) {
            NSError *reason = notification.userInfo[AVCaptureSessionErrorKey];
            [weak cameraFailed:reason ?: recording_error(@"The camera capture session stopped unexpectedly.")];
        }];
    return YES;
}

- (void)activateCamera {
    AVCaptureSession *session = _cameraSession;
    dispatch_async(_cameraControl, ^{
        [session startRunning];
        if (!session.isRunning) [self cameraFailed:recording_error(@"The camera did not start.")];
    });
}

- (BOOL)stopCamera {
    AVCaptureSession *session = _cameraSession;
    if (session == nil) return YES;
    AVCaptureVideoDataOutput *output = _cameraOutput;
    [[NSNotificationCenter defaultCenter] removeObserver:_cameraErrorObserver];
    _cameraErrorObserver = nil;
    dispatch_semaphore_t stopped = dispatch_semaphore_create(0);
    dispatch_async(_cameraControl, ^{
        [output setSampleBufferDelegate:nil queue:NULL];
        [session stopRunning];
        dispatch_semaphore_signal(stopped);
    });
    dispatch_sync(_queue, ^{
        self->_cameraSession = nil;
        self->_cameraOutput = nil;
    });
    return dispatch_semaphore_wait(stopped, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) == 0;
}

- (void)captureOutput:(AVCaptureOutput *)output didOutputSampleBuffer:(CMSampleBufferRef)sample
       fromConnection:(AVCaptureConnection *)connection {
    (void)output;
    (void)connection;
    if (_stopping) return;
    CMClockRef clock = _cameraSession.synchronizationClock;
    if (clock == NULL) return;
    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample);
    if (!CMTIME_IS_NUMERIC(pts)) return;
    CMTime hostPTS = CMSyncConvertTime(pts, clock, CMClockGetHostTimeClock());
    [self appendCamera:sample atHostTime:hostPTS];
}

- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sample ofType:(SCStreamOutputType)type {
    (void)stream;
    [self append:sample type:type];
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)reason {
    (void)stream;
    dispatch_async(_queue, ^{
        if (!self->_stopping && self->_failure == nil) self->_failure = reason;
    });
}

- (BOOL)acceptsSample:(CMSampleBufferRef)sample at:(CMTime)pts {
    return !_stopping && _failure == nil && !CMTIME_IS_NUMERIC(_pauseStart) &&
        CMSampleBufferDataIsReady(sample) && CMTIME_IS_NUMERIC(pts) &&
        (!CMTIME_IS_NUMERIC(_resumeCutoff) || CMTimeCompare(pts, _resumeCutoff) >= 0);
}

- (void)append:(CMSampleBufferRef)sample type:(SCStreamOutputType)type {
    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample);
    if (![self acceptsSample:sample at:pts]) return;
    BOOL screen = type == SCStreamOutputTypeScreen;
    if (screen) {
        NSArray *attachments = (__bridge NSArray *)CMSampleBufferGetSampleAttachmentsArray(sample, false);
        NSNumber *status = [attachments.firstObject objectForKey:SCStreamFrameInfoStatus];
        if (status == nil || status.integerValue != SCFrameStatusComplete ||
            CMSampleBufferGetImageBuffer(sample) == NULL) return;
    }
    if (!CMTIME_IS_NUMERIC(_first)) {
        if (!screen) return;
        _first = pts;
        _pausedSoFar = kCMTimeZero;
    }
    SpliceRecordingTrack *track = screen ? _screen : type == SCStreamOutputTypeMicrophone ? _microphone : _audio;
    if (track == nil) return;
    BOOL wrote = [track append:sample offset:CMTimeAdd(_first, _pausedSoFar)];
    if (screen) {
        if (wrote) _frames++;
        else _dropped++;
    }
    if (!wrote && track.writer.status == AVAssetWriterStatusFailed) {
        _failure = track.writer.error ?: recording_error(@"The recording encoder failed.");
    }
}

- (void)appendCamera:(CMSampleBufferRef)sample atHostTime:(CMTime)hostPTS {
    if (_cameraURL == nil || !CMTIME_IS_NUMERIC(_first) || ![self acceptsSample:sample at:hostPTS]) return;
    CMTime elapsed = CMTimeSubtract(hostPTS, CMTimeAdd(_first, _pausedSoFar));
    if (CMTimeCompare(elapsed, kCMTimeZero) < 0) return;
    CVImageBufferRef frame = CMSampleBufferGetImageBuffer(sample);
    if (frame == NULL) return;
    if (_camera == nil) {
        NSError *failure = nil;
        NSDictionary *settings = video_settings(CVPixelBufferGetWidth(frame), CVPixelBufferGetHeight(frame), @30);
        _camera = [[SpliceRecordingTrack alloc] initWithURL:_cameraURL video:YES settings:settings error:&failure];
        if (_camera == nil) {
            _failure = failure ?: recording_error(@"Could not create the camera writer.");
            return;
        }
    }
    CMTime offset = CMTimeSubtract(CMSampleBufferGetPresentationTimeStamp(sample), elapsed);
    if (![_camera append:sample offset:offset] && _camera.writer.status == AVAssetWriterStatusFailed) {
        _failure = _camera.writer.error ?: recording_error(@"The camera encoder failed.");
    }
}

- (void)setPaused:(BOOL)paused at:(CMTime)now {
    if (_stopping) return;
    if (paused && !CMTIME_IS_NUMERIC(_pauseStart)) {
        _pauseStart = now;
    } else if (!paused && CMTIME_IS_NUMERIC(_pauseStart)) {
        _pausedSoFar = CMTimeAdd(_pausedSoFar, positive(CMTimeSubtract(now, _pauseStart)));
        _pauseStart = kCMTimeInvalid;
        _resumeCutoff = now;
    }
}

- (CMTime)elapsedAt:(CMTime)now {
    if (CMTIME_IS_NUMERIC(_finalTime)) return _finalTime;
    if (!CMTIME_IS_NUMERIC(_first)) return kCMTimeZero;
    CMTime current = CMTIME_IS_NUMERIC(_pauseStart) ? _pauseStart : now;
    return positive(CMTimeSubtract(CMTimeSubtract(current, _first), _pausedSoFar));
}

- (SpliceRecordingStats)statsAt:(CMTime)now {
    return (SpliceRecordingStats){CMTimeGetSeconds([self elapsedAt:now]), _frames, _dropped};
}

- (NSError *)failure {
    return _failure;
}

- (void)detach {
    if (_stream == nil) return;
    [_stream removeStreamOutput:self type:SCStreamOutputTypeScreen error:NULL];
    if (_microphone != nil) [_stream removeStreamOutput:self type:SCStreamOutputTypeMicrophone error:NULL];
    if (_audio != nil) [_stream removeStreamOutput:self type:SCStreamOutputTypeAudio error:NULL];
    _stream = nil;
}

- (NSError *)finishAt:(CMTime)now {
    dispatch_group_t finishing = dispatch_group_create();
    NSMutableArray<SpliceRecordingTrack *> *tracks = [NSMutableArray arrayWithObject:_screen];
    if (_microphone != nil) [tracks addObject:_microphone];
    if (_audio != nil) [tracks addObject:_audio];
    __block NSError *failure;
    dispatch_sync(_queue, ^{
        self->_finalTime = [self elapsedAt:now];
        self->_stopping = YES;
        failure = self->_failure;
        if (self->_camera != nil) [tracks addObject:self->_camera];
        else if (self->_cameraURL != nil && failure == nil) {
            failure = recording_error(@"No camera frames arrived during the recording.");
        }
    });
    if (![self stopCamera] && failure == nil) failure = recording_error(@"The camera did not stop.");
    if (_stream != nil) {
        dispatch_semaphore_t stopped = dispatch_semaphore_create(0);
        __block NSError *stopFailure;
        [_stream stopCaptureWithCompletionHandler:^(NSError *reason) {
            stopFailure = reason;
            dispatch_semaphore_signal(stopped);
        }];
        bool timed_out = dispatch_semaphore_wait(stopped, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0;
        if (failure == nil) failure = timed_out ?
            recording_error(@"ScreenCaptureKit did not stop the stream.") : stopFailure;
    }
    dispatch_sync(_queue, ^{
        [self detach];
        if (failure != nil) return;
        if (self->_frames == 0 || CMTimeCompare(self->_finalTime, kCMTimeZero) <= 0) {
            failure = recording_error(@"The recording had no complete screen frames.");
            return;
        }
        for (SpliceRecordingTrack *track in tracks) {
            NSError *reason = [track sealAt:self->_finalTime group:finishing];
            if (reason != nil && failure == nil) failure = reason;
        }
    });
    if (dispatch_group_wait(finishing, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0 && failure == nil) {
        failure = recording_error(@"The capture files did not finish writing.");
    }
    if (failure == nil) {
        for (SpliceRecordingTrack *track in tracks) {
            if (track.writer.status != AVAssetWriterStatusCompleted) {
                failure = track.writer.error ?: recording_error(@"A capture file did not finish writing.");
                break;
            }
        }
    }
    if (failure != nil) [self cancel];
    return failure;
}

- (void)cancel {
    dispatch_sync(_queue, ^{
        self->_stopping = YES;
        [self->_stream stopCaptureWithCompletionHandler:nil];
        [self detach];
        [self->_screen cancel];
        [self->_microphone cancel];
        [self->_audio cancel];
        [self->_camera cancel];
    });
    [self stopCamera];
}
@end

void *splice_recording_start(const SpliceCaptureTarget *target, const char *directory, const char *microphone,
                             bool system_audio, bool camera, bool *denied, char **error) {
    @autoreleasepool {
        NSURL *root = [NSURL fileURLWithFileSystemRepresentation:directory isDirectory:YES relativeToURL:nil];
        SCStreamConfiguration *configuration = [SCStreamConfiguration new];
        SCContentFilter *filter = splice_capture_prepare(target, configuration, denied, error);
        if (filter == nil) return NULL;
        configuration.width = configuration.width & ~(size_t)1;
        configuration.height = configuration.height & ~(size_t)1;
        configuration.minimumFrameInterval = CMTimeMake(1, 60);
        configuration.queueDepth = 6;
        configuration.pixelFormat = target->kind == SPLICE_CAPTURE_WINDOW ?
            kCVPixelFormatType_32BGRA : kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
        configuration.ignoreShadowsSingleWindow = YES;
        configuration.capturesAudio = system_audio;
        configuration.excludesCurrentProcessAudio = YES;
        configuration.captureMicrophone = microphone != NULL;
        if (microphone != NULL && *microphone != '\0') {
            configuration.microphoneCaptureDeviceID = [NSString stringWithUTF8String:microphone];
        }
        configuration.sampleRate = 48000;
        configuration.channelCount = 2;
        NSError *failure = nil;
        SpliceRecording *recording = [[SpliceRecording alloc] initWithDirectory:root width:configuration.width
            height:configuration.height microphone:configuration.captureMicrophone audio:system_audio camera:camera error:&failure];
        if (recording == nil) {
            splice_capture_error(failure, "Could not configure the capture writers.", denied, error);
            return NULL;
        }
        if (camera) {
            if ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo] != AVAuthorizationStatusAuthorized) {
                *denied = true;
                splice_set_error(error, "Camera permission is unavailable.");
                return NULL;
            }
            if (![recording prepareCamera:&failure]) {
                splice_capture_error(failure, "Could not configure the camera.", denied, error);
                return NULL;
            }
        }
        SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:configuration delegate:recording];
        recording.stream = stream;
        if (![stream addStreamOutput:recording type:SCStreamOutputTypeScreen sampleHandlerQueue:recording.queue error:&failure] ||
            (system_audio && ![stream addStreamOutput:recording type:SCStreamOutputTypeAudio
                                                   sampleHandlerQueue:recording.queue error:&failure]) ||
            (microphone != NULL && ![stream addStreamOutput:recording type:SCStreamOutputTypeMicrophone
                                                          sampleHandlerQueue:recording.queue error:&failure])) {
            [recording cancel];
            splice_capture_error(failure, "Could not configure ScreenCaptureKit outputs.", denied, error);
            return NULL;
        }
        dispatch_semaphore_t ready = dispatch_semaphore_create(0);
        __block NSError *startFailure;
        [stream startCaptureWithCompletionHandler:^(NSError *reason) {
            startFailure = reason;
            dispatch_semaphore_signal(ready);
        }];
        if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
            [recording cancel];
            splice_set_error(error, "ScreenCaptureKit did not start the stream.");
            return NULL;
        }
        if (startFailure != nil) {
            [recording cancel];
            splice_capture_error(startFailure, "ScreenCaptureKit could not start recording.", denied, error);
            return NULL;
        }
        if (camera) [recording activateCamera];
        return (__bridge_retained void *)recording;
    }
}

bool splice_recording_status(void *handle, SpliceRecordingStats *out, bool *denied, char **error) {
    @autoreleasepool {
        SpliceRecording *recording = (__bridge SpliceRecording *)handle;
        __block NSError *failure;
        dispatch_sync(recording.queue, ^{
            *out = [recording statsAt:host_time()];
            failure = [recording failure];
        });
        if (failure != nil) splice_capture_error(failure, "Recording stopped unexpectedly.", denied, error);
        return failure == nil;
    }
}

void splice_recording_pause(void *handle, bool pause) {
    SpliceRecording *recording = (__bridge SpliceRecording *)handle;
    dispatch_sync(recording.queue, ^{ [recording setPaused:pause at:host_time()]; });
}

bool splice_recording_finish(void *handle, SpliceRecordingStats *out, bool *denied, char **error) {
    @autoreleasepool {
        SpliceRecording *recording = (__bridge SpliceRecording *)handle;
        NSError *failure = [recording finishAt:host_time()];
        if (failure != nil) {
            splice_capture_error(failure, "Could not finish recording.", denied, error);
            return false;
        }
        dispatch_sync(recording.queue, ^{ *out = [recording statsAt:host_time()]; });
        return true;
    }
}

void splice_recording_release(void *handle) {
    @autoreleasepool {
        SpliceRecording *recording = (__bridge_transfer SpliceRecording *)handle;
        [recording cancel];
    }
}
