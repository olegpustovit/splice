#ifndef SPLICE_RECORDING_INTERNAL_H
#define SPLICE_RECORDING_INTERNAL_H

#include "capture_internal.h"
#import <AVFoundation/AVFoundation.h>

@interface SpliceRecording : NSObject <SCStreamOutput, SCStreamDelegate>
@property(nonatomic, readonly) dispatch_queue_t queue;
@property(nonatomic, strong) SCStream *stream;
- (instancetype)initWithDirectory:(NSURL *)directory width:(NSInteger)width height:(NSInteger)height
                        microphone:(BOOL)microphone audio:(BOOL)audio error:(NSError **)error;
- (void)append:(CMSampleBufferRef)sample type:(SCStreamOutputType)type;
- (void)setPaused:(BOOL)paused at:(CMTime)now;
- (SpliceRecordingStats)statsAt:(CMTime)now;
- (NSError *)failure;
- (NSError *)finishAt:(CMTime)now;
- (void)cancel;
@end

#endif
