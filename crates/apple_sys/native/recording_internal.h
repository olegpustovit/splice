#ifndef SPLICE_RECORDING_INTERNAL_H
#define SPLICE_RECORDING_INTERNAL_H

#include "capture_internal.h"
#import <AVFoundation/AVFoundation.h>

@interface SpliceRecording : NSObject <SCStreamOutput, SCStreamDelegate, AVCaptureVideoDataOutputSampleBufferDelegate>
@property(nonatomic, readonly) dispatch_queue_t queue;
@property(nonatomic, strong) SCStream *stream;
// Only the requested apps' sound (record.start {systemAudio:{apps}}); nil otherwise.
@property(nonatomic, strong) SCStream *audioStream;
- (instancetype)initWithDirectory:(NSURL *)directory width:(NSInteger)width height:(NSInteger)height
                        microphone:(BOOL)microphone audio:(BOOL)audio camera:(BOOL)camera error:(NSError **)error;
- (BOOL)prepareCamera:(NSString *)identifier error:(NSError **)error;
- (void)activateCamera;
- (void)append:(CMSampleBufferRef)sample type:(SCStreamOutputType)type;
- (void)appendCamera:(CMSampleBufferRef)sample atHostTime:(CMTime)hostPTS;
- (void)setPaused:(BOOL)paused at:(CMTime)now;
- (SpliceRecordingStats)statsAt:(CMTime)now;
- (NSError *)failure;
- (NSError *)finishAt:(CMTime)now;
- (void)cancel;
@end

#endif
