#include "recording_internal.h"
#include "input_events.h"

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

static uint64_t nanoseconds(CMTime time) {
    if (!CMTIME_IS_NUMERIC(time)) return 0;
    int64_t value = CMTimeConvertScale(time, 1000000000, kCMTimeRoundingMethod_RoundTowardZero).value;
    return (uint64_t)MAX(0, value);
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
    // Fragments every 2 s keep a take readable up to the last fragment after a crash (T22.4).
    _writer.movieFragmentInterval = CMTimeMakeWithSeconds(2, 600);
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

// A complete screen or window frame with an image (idle and blank frames carry none).
static BOOL complete_frame(CMSampleBufferRef sample) {
    NSArray *attachments = (__bridge NSArray *)CMSampleBufferGetSampleAttachmentsArray(sample, false);
    NSNumber *status = [attachments.firstObject objectForKey:SCStreamFrameInfoStatus];
    return status != nil && status.integerValue == SCFrameStatusComplete && CMSampleBufferGetImageBuffer(sample) != NULL;
}

// The most extra windows one take records (T46.6: 2-4 windows in all).
#define SPLICE_EXTRA_WINDOWS 3

@implementation SpliceRecording {
    SpliceRecordingTrack *_screen, *_microphone, *_audio, *_camera;
    // Multi-window takes (T46.6): each other window's stream and writer, and the newest frame
    // a window delivered before the take's first screen frame (written at zero once it starts).
    NSMutableArray<SCStream *> *_windowStreams;
    NSMutableArray<SpliceRecordingTrack *> *_windows;
    CMSampleBufferRef _windowPending[SPLICE_EXTRA_WINDOWS];
    NSURL *_cameraURL;
    AVCaptureSession *_cameraSession;
    AVCaptureVideoDataOutput *_cameraOutput;
    dispatch_queue_t _cameraControl;
    id _cameraErrorObserver;
    SpliceInputMonitor *_inputMonitor;
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
    _windowStreams = [NSMutableArray array];
    _windows = [NSMutableArray array];
    return self;
}

- (BOOL)addWindow:(SCStream *)stream url:(NSURL *)url width:(NSInteger)width height:(NSInteger)height
            error:(NSError **)error {
    if (_windows.count >= SPLICE_EXTRA_WINDOWS) {
        *error = recording_error(@"A take records at most four windows.");
        return NO;
    }
    SpliceRecordingTrack *track = [[SpliceRecordingTrack alloc] initWithURL:url video:YES
                                                                    settings:video_settings(width, height, @60) error:error];
    if (track == nil) return NO;
    [_windowStreams addObject:stream];
    [_windows addObject:track];
    return YES;
}

// Writes window index's held frame at zero when its movie has none yet: a window that never
// changes after the take starts still has its picture from the first frame on.
- (void)flushWindow:(NSUInteger)index {
    CMSampleBufferRef pending = _windowPending[index];
    if (pending == NULL) return;
    _windowPending[index] = NULL;
    SpliceRecordingTrack *track = _windows[index];
    if (track.count == 0 && ![track append:pending offset:CMSampleBufferGetPresentationTimeStamp(pending)] &&
        track.writer.status == AVAssetWriterStatusFailed && _failure == nil) {
        _failure = track.writer.error ?: recording_error(@"A window encoder failed.");
    }
    CFRelease(pending);
}

- (void)appendWindow:(CMSampleBufferRef)sample index:(NSUInteger)index {
    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample);
    if (_stopping || _failure != nil || !CMTIME_IS_NUMERIC(pts) || !CMSampleBufferDataIsReady(sample) ||
        !complete_frame(sample)) return;
    if (!CMTIME_IS_NUMERIC(_first)) {
        CFRetain(sample);
        if (_windowPending[index] != NULL) CFRelease(_windowPending[index]);
        _windowPending[index] = sample;
        return;
    }
    if (![self acceptsSample:sample at:pts]) return;
    [self flushWindow:index];
    SpliceRecordingTrack *track = _windows[index];
    // A window's first frame after the take starts opens its movie at zero.
    CMTime offset = track.count == 0 ? pts : CMTimeAdd(_first, _pausedSoFar);
    if (![track append:sample offset:offset] && track.writer.status == AVAssetWriterStatusFailed) {
        _failure = track.writer.error ?: recording_error(@"A window encoder failed.");
    }
}

- (void)cameraFailed:(NSError *)error {
    dispatch_async(_queue, ^{
        if (!self->_stopping && self->_failure == nil) self->_failure = error;
    });
}

- (BOOL)prepareCamera:(NSString *)identifier error:(NSError **)error {
    AVCaptureDevice *device = identifier.length > 0 ? [AVCaptureDevice deviceWithUniqueID:identifier]
                                                    : [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
    if (device == nil) {
        *error = recording_error(identifier.length > 0 ? @"The requested camera is not connected."
                                                       : @"No default camera is available.");
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

- (BOOL)prepareInput:(SpliceCaptureGeometry)geometry windows:(const SpliceCaptureGeometry *)windows
                count:(size_t)count denied:(bool *)denied error:(char **)error {
    if (!CGPreflightListenEventAccess()) {
        *denied = true;
        splice_set_error(error, "Input Monitoring permission is unavailable.");
        return NO;
    }
    _inputMonitor = [[SpliceInputMonitor alloc] initWithGeometry:geometry error:error];
    if (_inputMonitor == nil) return NO;
    [_inputMonitor watchWindows:windows count:count];
    if (![_inputMonitor start]) {
        *denied = !CGPreflightListenEventAccess();
        splice_set_error(error, "Could not start the listen-only input monitor.");
        return NO;
    }
    return YES;
}

- (void)publishInputClock {
    [_inputMonitor updateClock:(SpliceInputClock){
        .first = nanoseconds(_first), .paused = nanoseconds(_pausedSoFar),
        .resumed = nanoseconds(_resumeCutoff),
        .active = !_stopping && CMTIME_IS_NUMERIC(_first) && !CMTIME_IS_NUMERIC(_pauseStart),
    }];
}

- (size_t)readInput:(SpliceEvent *)out capacity:(size_t)capacity {
    return [_inputMonitor read:out capacity:capacity];
}

- (uint64_t)droppedInput {
    return [_inputMonitor dropped];
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
    if (stream != _stream && type == SCStreamOutputTypeScreen) {
        NSUInteger index = [_windowStreams indexOfObjectIdenticalTo:stream];
        if (index != NSNotFound) [self appendWindow:sample index:index];
        return;
    }
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
    if (screen && !complete_frame(sample)) return;
    if (!CMTIME_IS_NUMERIC(_first)) {
        if (!screen) return;
        _first = pts;
        _pausedSoFar = kCMTimeZero;
        [self publishInputClock];
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
    [self publishInputClock];
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

- (BOOL)inputStateAt:(CMTime)now out:(SpliceCaptureInputState *)out {
    if (_inputMonitor == nil || !CMTIME_IS_NUMERIC(_first)) return NO;
    SpliceCaptureGeometry geometry = [_inputMonitor geometry];
    *out = (SpliceCaptureInputState){
        .rect = geometry.rect, .scale = geometry.scale, .kind = geometry.kind,
        .origin_ns = nanoseconds(_first), .elapsed_ns = nanoseconds([self elapsedAt:now]),
        .active = !_stopping && !CMTIME_IS_NUMERIC(_pauseStart),
    };
    return YES;
}

- (NSError *)failure {
    if (_failure == nil && [_inputMonitor wasDisabled]) {
        return recording_error(@"The input monitor was disabled during the recording.");
    }
    return _failure;
}

- (void)detach {
    for (SCStream *window in _windowStreams) {
        [window removeStreamOutput:self type:SCStreamOutputTypeScreen error:NULL];
    }
    [_windowStreams removeAllObjects];
    if (_audioStream != nil) {
        [_audioStream removeStreamOutput:self type:SCStreamOutputTypeAudio error:NULL];
        _audioStream = nil;
    }
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
        [self publishInputClock];
        failure = [self failure];
        if (self->_camera != nil) [tracks addObject:self->_camera];
        else if (self->_cameraURL != nil && failure == nil) {
            failure = recording_error(@"No camera frames arrived during the recording.");
        }
    });
    if (_inputMonitor != nil && ![_inputMonitor stop] && failure == nil) {
        failure = recording_error(@"The input monitor did not stop.");
    }
    if (![self stopCamera] && failure == nil) failure = recording_error(@"The camera did not stop.");
    for (SCStream *window in _windowStreams) {
        dispatch_semaphore_t stopped = dispatch_semaphore_create(0);
        [window stopCaptureWithCompletionHandler:^(NSError *reason) {
            (void)reason;
            dispatch_semaphore_signal(stopped);
        }];
        if (dispatch_semaphore_wait(stopped, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0 && failure == nil) {
            failure = recording_error(@"ScreenCaptureKit did not stop a window stream.");
        }
    }
    if (_audioStream != nil) {
        dispatch_semaphore_t stopped = dispatch_semaphore_create(0);
        [_audioStream stopCaptureWithCompletionHandler:^(NSError *reason) {
            (void)reason;
            dispatch_semaphore_signal(stopped);
        }];
        if (dispatch_semaphore_wait(stopped, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0 && failure == nil) {
            failure = recording_error(@"ScreenCaptureKit did not stop the app audio stream.");
        }
    }
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
        for (NSUInteger index = 0; index < self->_windows.count; ++index) {
            [self flushWindow:index];
            if (self->_windows[index].count == 0 && failure == nil) {
                failure = recording_error(@"A recorded window delivered no complete frames.");
            }
            [tracks addObject:self->_windows[index]];
        }
        if (failure != nil) return;
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

- (void)dealloc {
    for (NSUInteger index = 0; index < SPLICE_EXTRA_WINDOWS; ++index) {
        if (_windowPending[index] != NULL) CFRelease(_windowPending[index]);
    }
}

- (void)cancel {
    dispatch_sync(_queue, ^{
        self->_stopping = YES;
        [self publishInputClock];
        [self->_stream stopCaptureWithCompletionHandler:nil];
        [self->_audioStream stopCaptureWithCompletionHandler:nil];
        for (SCStream *window in self->_windowStreams) [window stopCaptureWithCompletionHandler:nil];
        for (SpliceRecordingTrack *window in self->_windows) [window cancel];
        [self detach];
        [self->_screen cancel];
        [self->_microphone cancel];
        [self->_audio cancel];
        [self->_camera cancel];
    });
    [_inputMonitor stop];
    [self stopCamera];
}
@end

void *splice_recording_start(const SpliceCaptureTarget *target, const SpliceCaptureTarget *windows,
                             size_t window_count, const char *directory, const char *microphone,
                             bool system_audio, const char *audio_apps, const char *camera_id, bool events,
                             bool *denied, char **error) {
    @autoreleasepool {
        const bool camera = camera_id != NULL;
        // Only these apps' sound: a second, audio-only stream keeps the app filter off the video
        // (Recorder's design), so the screen shows everything the source selects.
        SCContentFilter *audio_filter = nil;
        if (system_audio && audio_apps != NULL) {
            NSMutableArray<NSString *> *apps = [NSMutableArray array];
            for (NSString *name in [[NSString stringWithUTF8String:audio_apps] componentsSeparatedByString:@"\n"]) {
                if (name.length > 0) [apps addObject:name];
            }
            audio_filter = splice_capture_audio_filter(apps, denied, error);
            if (audio_filter == nil) return NULL;
        }
        NSURL *root = [NSURL fileURLWithFileSystemRepresentation:directory isDirectory:YES relativeToURL:nil];
        SCStreamConfiguration *configuration = [SCStreamConfiguration new];
        SpliceCaptureGeometry geometry;
        SCContentFilter *filter = splice_capture_prepare(target, configuration, &geometry, denied, error);
        if (filter == nil) return NULL;
        configuration.width = configuration.width & ~(size_t)1;
        configuration.height = configuration.height & ~(size_t)1;
        configuration.minimumFrameInterval = CMTimeMake(1, 60);
        configuration.queueDepth = 6;
        configuration.pixelFormat = target->kind == SPLICE_CAPTURE_WINDOW ?
            kCVPixelFormatType_32BGRA : kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
        configuration.ignoreShadowsSingleWindow = YES;
        configuration.capturesAudio = system_audio && audio_filter == nil;
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
            if (![recording prepareCamera:[NSString stringWithUTF8String:camera_id] error:&failure]) {
                splice_capture_error(failure, "Could not configure the camera.", denied, error);
                return NULL;
            }
        }
        if (window_count > SPLICE_EXTRA_WINDOWS) {
            splice_set_error(error, "A take records at most four windows.");
            return NULL;
        }
        // Every other window: its own stream and movie on the take's clock (T46.6).
        NSMutableArray<SCStream *> *extra = [NSMutableArray array];
        SpliceCaptureGeometry placed[SPLICE_EXTRA_WINDOWS];
        for (size_t index = 0; index < window_count; ++index) {
            SCStreamConfiguration *picture = [SCStreamConfiguration new];
            SCContentFilter *window = splice_capture_prepare(&windows[index], picture, &placed[index], denied, error);
            if (window == nil) return NULL;
            picture.width = picture.width & ~(size_t)1;
            picture.height = picture.height & ~(size_t)1;
            picture.minimumFrameInterval = CMTimeMake(1, 60);
            picture.queueDepth = 6;
            picture.pixelFormat = kCVPixelFormatType_32BGRA;
            picture.ignoreShadowsSingleWindow = YES;
            picture.capturesAudio = NO;
            SCStream *stream = [[SCStream alloc] initWithFilter:window configuration:picture delegate:recording];
            NSURL *url = [root URLByAppendingPathComponent:[NSString stringWithFormat:@"window-%zu.mov", index + 1]];
            if (![recording addWindow:stream url:url width:picture.width height:picture.height error:&failure] ||
                ![stream addStreamOutput:recording type:SCStreamOutputTypeScreen sampleHandlerQueue:recording.queue
                                   error:&failure]) {
                [recording cancel];
                splice_capture_error(failure, "Could not configure a window stream.", denied, error);
                return NULL;
            }
            [extra addObject:stream];
        }
        if (events && ![recording prepareInput:geometry windows:placed count:window_count denied:denied error:error]) {
            [recording cancel];
            return NULL;
        }
        SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:configuration delegate:recording];
        recording.stream = stream;
        if (audio_filter != nil) {
            SCStreamConfiguration *sound = [SCStreamConfiguration new];
            sound.width = 2;
            sound.height = 2;
            sound.minimumFrameInterval = CMTimeMake(1, 1);
            sound.capturesAudio = YES;
            sound.excludesCurrentProcessAudio = YES;
            sound.sampleRate = 48000;
            sound.channelCount = 2;
            recording.audioStream = [[SCStream alloc] initWithFilter:audio_filter configuration:sound delegate:recording];
            if (![recording.audioStream addStreamOutput:recording type:SCStreamOutputTypeAudio
                                     sampleHandlerQueue:recording.queue error:&failure]) {
                [recording cancel];
                splice_capture_error(failure, "Could not configure the app audio stream.", denied, error);
                return NULL;
            }
        }
        if (![stream addStreamOutput:recording type:SCStreamOutputTypeScreen sampleHandlerQueue:recording.queue error:&failure] ||
            (system_audio && audio_filter == nil && ![stream addStreamOutput:recording type:SCStreamOutputTypeAudio
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
        if (startFailure == nil && recording.audioStream != nil) {
            dispatch_semaphore_t sound = dispatch_semaphore_create(0);
            [recording.audioStream startCaptureWithCompletionHandler:^(NSError *reason) {
                startFailure = reason;
                dispatch_semaphore_signal(sound);
            }];
            if (dispatch_semaphore_wait(sound, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
                [recording cancel];
                splice_set_error(error, "ScreenCaptureKit did not start the app audio stream.");
                return NULL;
            }
        }
        for (SCStream *window in extra) {
            if (startFailure != nil) break;
            dispatch_semaphore_t started = dispatch_semaphore_create(0);
            [window startCaptureWithCompletionHandler:^(NSError *reason) {
                startFailure = reason;
                dispatch_semaphore_signal(started);
            }];
            if (dispatch_semaphore_wait(started, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) != 0) {
                [recording cancel];
                splice_set_error(error, "ScreenCaptureKit did not start a window stream.");
                return NULL;
            }
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

size_t splice_recording_read_input(void *handle, SpliceEvent *out, size_t capacity) {
    SpliceRecording *recording = (__bridge SpliceRecording *)handle;
    return [recording readInput:out capacity:capacity];
}

uint64_t splice_recording_dropped_input(void *handle) {
    SpliceRecording *recording = (__bridge SpliceRecording *)handle;
    return [recording droppedInput];
}

bool splice_recording_input_state(void *handle, SpliceCaptureInputState *out) {
    SpliceRecording *recording = (__bridge SpliceRecording *)handle;
    __block BOOL available;
    dispatch_sync(recording.queue, ^{ available = [recording inputStateAt:host_time() out:out]; });
    return available;
}
