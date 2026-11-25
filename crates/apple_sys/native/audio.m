// Offline audio (PLAN.md T19): decode a track to PCM, run the effect chain through AVAudioEngine
// manual rendering, measure envelopes, and classify sounds. Everything runs headless.
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <SoundAnalysis/SoundAnalysis.h>
#include "platform.h"

#include <float.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

// The synchronous track accessors are fine off the main thread in this headless path.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static const double kRate = 48000.0;

// Decodes `track` of `path` to deinterleaved float stereo at 48 kHz.
static AVAudioPCMBuffer *decode(const char *path, uint32_t track, char **error) {
    NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    NSArray<AVAssetTrack *> *tracks = [asset tracksWithMediaType:AVMediaTypeAudio];
    if (track >= tracks.count) {
        splice_set_error(error, "The file has no such audio track.");
        return nil;
    }
    NSError *failure = nil;
    AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&failure];
    NSDictionary *settings = @{
        AVFormatIDKey: @(kAudioFormatLinearPCM), AVSampleRateKey: @(kRate), AVNumberOfChannelsKey: @2,
        AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES, AVLinearPCMIsNonInterleaved: @NO,
        AVLinearPCMIsBigEndianKey: @NO,
    };
    AVAssetReaderTrackOutput *output = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:tracks[track]
                                                                                 outputSettings:settings];
    if (reader == nil || ![reader canAddOutput:output]) {
        splice_set_error(error, "Could not read the audio track.");
        return nil;
    }
    [reader addOutput:output];
    [reader startReading];
    NSMutableData *samples = [NSMutableData data];
    CMSampleBufferRef buffer;
    while ((buffer = [output copyNextSampleBuffer]) != NULL) {
        CMBlockBufferRef block = CMSampleBufferGetDataBuffer(buffer);
        size_t length = CMBlockBufferGetDataLength(block);
        NSUInteger at = samples.length;
        [samples increaseLengthBy:length];
        CMBlockBufferCopyDataBytes(block, 0, length, (uint8_t *)samples.mutableBytes + at);
        CFRelease(buffer);
    }
    if (reader.status == AVAssetReaderStatusFailed) {
        splice_set_error(error, reader.error.localizedDescription.UTF8String);
        return nil;
    }
    AVAudioFormat *format = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:kRate channels:2];
    AVAudioFrameCount frames = (AVAudioFrameCount)(samples.length / (sizeof(float) * 2));
    AVAudioPCMBuffer *pcm = [[AVAudioPCMBuffer alloc] initWithPCMFormat:format frameCapacity:MAX(frames, 1)];
    pcm.frameLength = frames;
    const float *interleaved = samples.bytes;
    for (AVAudioFrameCount i = 0; i < frames; i++) {
        pcm.floatChannelData[0][i] = interleaved[2 * i];
        pcm.floatChannelData[1][i] = interleaved[2 * i + 1];
    }
    return pcm;
}

static AVAudioUnitEffect *effect(OSType subtype, char **error) {
    AudioComponentDescription description = {kAudioUnitType_Effect, subtype, kAudioUnitManufacturer_Apple, 0, 0};
    AVAudioUnitEffect *unit = [[AVAudioUnitEffect alloc] initWithAudioComponentDescription:description];
    if (unit == nil) splice_set_error(error, "An audio effect is unavailable on this Mac.");
    return unit;
}

static void set(AVAudioUnit *unit, AudioUnitParameterID id, float value) {
    AudioUnitSetParameter(unit.audioUnit, id, kAudioUnitScope_Global, 0, value, 0);
}

bool splice_audio_render(const char *input, uint32_t track, const char *output, const SpliceAudioChain *chain,
                         char **error) {
    @autoreleasepool {
        AVAudioPCMBuffer *source = decode(input, track, error);
        if (source == nil) return false;
        AVAudioEngine *engine = [AVAudioEngine new];
        AVAudioPlayerNode *player = [AVAudioPlayerNode new];
        [engine attachNode:player];
        NSMutableArray<AVAudioNode *> *nodes = [NSMutableArray arrayWithObject:player];
        if (chain->eq) {
            AVAudioUnitEQ *eq = [[AVAudioUnitEQ alloc] initWithNumberOfBands:4];
            AVAudioUnitEQFilterParameters *cut = eq.bands[0];
            cut.filterType = AVAudioUnitEQFilterTypeHighPass;
            cut.frequency = chain->low_cut;
            cut.bypass = chain->low_cut <= 20.5;
            AVAudioUnitEQFilterParameters *low = eq.bands[1];
            low.filterType = AVAudioUnitEQFilterTypeLowShelf;
            low.frequency = 120;
            low.gain = chain->low;
            low.bypass = NO;
            AVAudioUnitEQFilterParameters *mid = eq.bands[2];
            mid.filterType = AVAudioUnitEQFilterTypeParametric;
            mid.frequency = chain->mid_frequency;
            mid.bandwidth = 1.0;
            mid.gain = chain->mid;
            mid.bypass = NO;
            AVAudioUnitEQFilterParameters *high = eq.bands[3];
            high.filterType = AVAudioUnitEQFilterTypeHighShelf;
            high.frequency = 8000;
            high.gain = chain->high;
            high.bypass = NO;
            [nodes addObject:eq];
        }
        if (chain->isolation > 0) {
            AVAudioUnitEffect *isolation = effect(kAudioUnitSubType_AUSoundIsolation, error);
            if (isolation == nil) return false;
            set(isolation, kAUSoundIsolationParam_WetDryMixPercent, (float)(chain->isolation * 100.0));
            [nodes addObject:isolation];
        }
        if (chain->gate || chain->compressor) {
            AVAudioUnitEffect *dynamics = effect(kAudioUnitSubType_DynamicsProcessor, error);
            if (dynamics == nil) return false;
            if (chain->compressor) {
                set(dynamics, kDynamicsProcessorParam_Threshold, (float)chain->threshold);
                // Headroom is the range above threshold that compresses into it: ratio → headroom.
                set(dynamics, kDynamicsProcessorParam_HeadRoom, (float)MAX(0.1, 30.0 / MAX(chain->ratio, 1.0)));
                set(dynamics, kDynamicsProcessorParam_OverallGain, (float)chain->makeup);
            } else {
                set(dynamics, kDynamicsProcessorParam_Threshold, 0);
                set(dynamics, kDynamicsProcessorParam_HeadRoom, 40);
            }
            if (chain->gate) {
                set(dynamics, kDynamicsProcessorParam_ExpansionThreshold, (float)chain->gate_threshold);
                set(dynamics, kDynamicsProcessorParam_ExpansionRatio, 50);
            }
            [nodes addObject:dynamics];
        }
        AVAudioFormat *format = source.format;
        for (NSUInteger i = 1; i < nodes.count; i++) [engine attachNode:nodes[i]];
        for (NSUInteger i = 0; i + 1 < nodes.count; i++) [engine connect:nodes[i] to:nodes[i + 1] format:format];
        [engine connect:nodes.lastObject to:engine.mainMixerNode format:format];
        player.pan = (float)chain->pan;
        NSError *failure = nil;
        if (![engine enableManualRenderingMode:AVAudioEngineManualRenderingModeOffline format:format
                             maximumFrameCount:4096 error:&failure] || ![engine startAndReturnError:&failure]) {
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "Could not start offline rendering.");
            return false;
        }
        [player scheduleBuffer:source completionHandler:nil];
        [player play];
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:output isDirectory:NO relativeToURL:nil];
        AVAudioFile *file = [[AVAudioFile alloc] initForWriting:url settings:format.settings
                                                   commonFormat:AVAudioPCMFormatFloat32 interleaved:NO error:&failure];
        if (file == nil) {
            [engine stop];
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "Could not write the rendered audio.");
            return false;
        }
        AVAudioPCMBuffer *block = [[AVAudioPCMBuffer alloc] initWithPCMFormat:engine.manualRenderingFormat
                                                                frameCapacity:engine.manualRenderingMaximumFrameCount];
        AVAudioFramePosition total = source.frameLength;
        while (engine.manualRenderingSampleTime < total) {
            AVAudioFrameCount want = (AVAudioFrameCount)MIN((AVAudioFramePosition)block.frameCapacity,
                                                            total - engine.manualRenderingSampleTime);
            AVAudioEngineManualRenderingStatus status = [engine renderOffline:want toBuffer:block error:&failure];
            if (status == AVAudioEngineManualRenderingStatusError ||
                (status == AVAudioEngineManualRenderingStatusSuccess && ![file writeFromBuffer:block error:&failure])) {
                [engine stop];
                splice_set_error(error, failure.localizedDescription.UTF8String ?: "Offline rendering failed.");
                return false;
            }
        }
        [engine stop];
        return true;
    }
}

float *splice_audio_envelope(const char *input, uint32_t track, double rate, size_t *count, char **error) {
    @autoreleasepool {
        AVAudioPCMBuffer *pcm = decode(input, track, error);
        if (pcm == nil || rate <= 0) return NULL;
        size_t hop = (size_t)MAX(1.0, kRate / rate);
        size_t n = pcm.frameLength / hop;
        float *out = calloc(MAX(n, 1), sizeof(float));
        for (size_t i = 0; i < n; i++) {
            double sum = 0;
            for (size_t j = i * hop; j < (i + 1) * hop; j++) {
                float m = 0.5f * (pcm.floatChannelData[0][j] + pcm.floatChannelData[1][j]);
                sum += m * m;
            }
            out[i] = (float)sqrt(sum / hop);
        }
        *count = n;
        return out;
    }
}

static bool waveform_buffer(CMSampleBufferRef sample, double bucket_rate, size_t buckets, double *energy) {
    CMItemCount count = CMSampleBufferGetNumSamples(sample);
    if (count == 0) return true;  // Empty edits carry no PCM.
    CMFormatDescriptionRef description = CMSampleBufferGetFormatDescription(sample);
    const AudioStreamBasicDescription *format = description != NULL &&
        CMFormatDescriptionGetMediaType(description) == kCMMediaType_Audio
        ? CMAudioFormatDescriptionGetStreamBasicDescription(description) : NULL;
    if (count < 0 || !CMSampleBufferDataIsReady(sample) || format == NULL ||
        format->mFormatID != kAudioFormatLinearPCM || format->mBitsPerChannel != 32 ||
        !(format->mFormatFlags & kAudioFormatFlagIsFloat) ||
        (format->mFormatFlags & (kAudioFormatFlagIsNonInterleaved | kAudioFormatFlagIsBigEndian)) ||
        !isfinite(format->mSampleRate) || format->mSampleRate <= 0) return false;

    size_t channels = format->mChannelsPerFrame;
    size_t frame_bytes = format->mBytesPerFrame;
    char scratch[16 * 1024];
    if (channels == 0 || frame_bytes != channels * sizeof(float) || frame_bytes > sizeof(scratch)) return false;
    CMBlockBufferRef block = CMSampleBufferGetDataBuffer(sample);
    if (block == NULL) return false;
    size_t length = CMBlockBufferGetDataLength(block);
    size_t frames = (size_t)count;
    if (length / frame_bytes != frames || length % frame_bytes != 0) return false;

    // Work in bucket units so missing time contributes zero power even within a partly filled bucket.
    double origin = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample)) * bucket_rate;
    double step = bucket_rate / format->mSampleRate;
    double end = origin + (double)frames * step;
    if (!isfinite(origin) || !isfinite(end) || step <= 0) return false;
    if (origin >= (double)buckets || end <= 0) return true;
    double channel_weight = 1.0 / (double)channels;
    for (size_t at = 0; at < frames;) {
        size_t chunk = MIN(frames - at, sizeof(scratch) / frame_bytes);
        char *pcm = NULL;
        if (CMBlockBufferAccessDataBytes(block, at * frame_bytes, chunk * frame_bytes, scratch, &pcm) != noErr ||
            pcm == NULL) return false;
        for (size_t i = 0; i < chunk; i++) {
            double from = MAX(0.0, origin + (double)(at + i) * step);
            double to = MIN((double)buckets, origin + (double)(at + i + 1) * step);
            if (from >= to) continue;
            double power = 0;
            for (size_t c = 0; c < channels; c++) {
                float value;
                memcpy(&value, pcm + i * frame_bytes + c * sizeof(value), sizeof(value));
                if (!isfinite(value)) return false;
                power += (double)value * value;
            }
            power *= channel_weight;
            for (size_t bucket = (size_t)from; from < to; bucket++) {
                double boundary = MIN(to, (double)(bucket + 1));
                energy[bucket] += power * (boundary - from);
                from = boundary;
            }
        }
        at += chunk;
    }
    return true;
}

bool splice_audio_waveform(const char *input, uint32_t track, double seconds, size_t buckets, float *out, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:input isDirectory:NO relativeToURL:nil];
        if (url == nil) {
            splice_set_error(error, "Could not open the local audio URL.");
            return false;
        }
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:@{AVURLAssetPreferPreciseDurationAndTimingKey: @YES}];
        NSArray<AVAssetTrack *> *tracks = [asset tracksWithMediaType:AVMediaTypeAudio];
        if (track >= tracks.count) {
            splice_set_error(error, "The file has no such audio track.");
            return false;
        }
        NSError *failure = nil;
        AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&failure];
        NSDictionary *settings = @{
            AVFormatIDKey: @(kAudioFormatLinearPCM), AVLinearPCMBitDepthKey: @32, AVLinearPCMIsFloatKey: @YES,
            AVLinearPCMIsNonInterleaved: @NO, AVLinearPCMIsBigEndianKey: @NO,
        };
        AVAssetReaderTrackOutput *output = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:tracks[track]
                                                                                     outputSettings:settings];
        if (reader == nil || output == nil || ![reader canAddOutput:output]) {
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "Could not read the audio track.");
            return false;
        }
        output.alwaysCopiesSampleData = NO;
        reader.timeRange = CMTimeRangeMake(kCMTimeZero, CMTimeMakeWithSeconds(seconds, 1000000000));
        [reader addOutput:output];
        if (![reader startReading]) {
            splice_set_error(error, reader.error.localizedDescription.UTF8String ?: "Could not start reading audio.");
            return false;
        }
        double *energy = calloc(buckets, sizeof(*energy));
        if (energy == NULL) {
            [reader cancelReading];
            splice_set_error(error, "Could not allocate audio waveform buckets.");
            return false;
        }
        bool ok = true;
        double bucket_rate = (double)buckets / seconds;
        for (;;) {
            @autoreleasepool {
                CMSampleBufferRef sample = [output copyNextSampleBuffer];
                if (sample == NULL) break;
                ok = waveform_buffer(sample, bucket_rate, buckets, energy);
                CFRelease(sample);
                if (!ok) break;
            }
        }
        if (!ok) {
            [reader cancelReading];
            splice_set_error(error, "The audio reader returned invalid PCM or timing.");
        } else if (reader.status != AVAssetReaderStatusCompleted) {
            splice_set_error(error, reader.error.localizedDescription.UTF8String ?: "Reading the audio failed.");
            ok = false;
        } else {
            for (size_t i = 0; i < buckets; i++) out[i] = (float)MIN(sqrt(energy[i]), (double)FLT_MAX);
        }
        free(energy);
        return ok;
    }
}

@interface SpliceSoundObserver : NSObject <SNResultsObserving>
@property NSMutableString *lines;
@end

@implementation SpliceSoundObserver
- (void)request:(id<SNRequest>)request didProduceResult:(id<SNResult>)result {
    SNClassificationResult *classification = (SNClassificationResult *)result;
    SNClassification *top = classification.classifications.firstObject;
    if (top == nil) return;
    [self.lines appendFormat:@"%.3f\t%.3f\t%@\t%.3f\n", CMTimeGetSeconds(classification.timeRange.start),
                             CMTimeGetSeconds(classification.timeRange.duration), top.identifier, top.confidence];
}
@end

char *splice_audio_classify(const char *input, uint32_t track, const char *scratch, char **error) {
    @autoreleasepool {
        SpliceAudioChain plain = {0};
        if (!splice_audio_render(input, track, scratch, &plain, error)) return NULL;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:scratch isDirectory:NO relativeToURL:nil];
        NSError *failure = nil;
        SNAudioFileAnalyzer *analyzer = [[SNAudioFileAnalyzer alloc] initWithURL:url error:&failure];
        SNClassifySoundRequest *request = [[SNClassifySoundRequest alloc] initWithClassifierIdentifier:SNClassifierIdentifierVersion1
                                                                                                error:&failure];
        if (analyzer == nil || request == nil) {
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "Sound analysis is unavailable.");
            return NULL;
        }
        request.windowDuration = CMTimeMakeWithSeconds(1.0, 48000);
        request.overlapFactor = 0.5;
        SpliceSoundObserver *observer = [SpliceSoundObserver new];
        observer.lines = [NSMutableString string];
        if (![analyzer addRequest:request withObserver:observer error:&failure]) {
            splice_set_error(error, failure.localizedDescription.UTF8String ?: "Sound analysis failed.");
            return NULL;
        }
        [analyzer analyze];
        [[NSFileManager defaultManager] removeItemAtURL:url error:nil];
        return strdup(observer.lines.UTF8String);
    }
}

#pragma clang diagnostic pop
