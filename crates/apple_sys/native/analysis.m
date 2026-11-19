// Vision analysis for agents (PLAN.md T20.1, T18.2–T18.3): shots, on-screen text, faces and
// people at sampled frames; region tracking for masks; a person-segmentation matte movie.
// Results are tab-separated lines; Rust parses them.
#import <AVFoundation/AVFoundation.h>
#import <CoreImage/CoreImage.h>
#import <Vision/Vision.h>
#include "platform.h"

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static AVAssetReaderTrackOutput *video_output(NSURL *url, AVAssetReader **reader_out, double *fps, CMTime *duration,
                                              char **error) {
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:@{AVURLAssetPreferPreciseDurationAndTimingKey: @YES}];
    AVAssetTrack *track = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
    if (track == nil) {
        splice_set_error(error, "The file has no video track.");
        return nil;
    }
    NSError *failure = nil;
    AVAssetReader *reader = [AVAssetReader assetReaderWithAsset:asset error:&failure];
    AVAssetReaderTrackOutput *output = [AVAssetReaderTrackOutput assetReaderTrackOutputWithTrack:track outputSettings:@{
        (NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
    }];
    output.alwaysCopiesSampleData = NO;
    if (reader == nil || ![reader canAddOutput:output]) {
        splice_set_error(error, failure.localizedDescription.UTF8String ?: "Could not read the video.");
        return nil;
    }
    [reader addOutput:output];
    if (![reader startReading]) {
        splice_set_error(error, reader.error.localizedDescription.UTF8String ?: "Could not read the video.");
        return nil;
    }
    *reader_out = reader;
    if (fps) *fps = track.nominalFrameRate > 0 ? track.nominalFrameRate : 30;
    if (duration) *duration = asset.duration;
    return output;
}

static void box_line(NSMutableString *out, const char *kind, double t, CGRect box, NSString *text) {
    // Vision boxes are normalized with a bottom-left origin; report top-left.
    [out appendFormat:@"%s\t%.4f\t%.4f\t%.4f\t%.4f\t%.4f", kind, t, box.origin.x,
                      1.0 - box.origin.y - box.size.height, box.size.width, box.size.height];
    if (text) [out appendFormat:@"\t%@", [[text stringByReplacingOccurrencesOfString:@"\t" withString:@" "]
                                            stringByReplacingOccurrencesOfString:@"\n" withString:@" "]];
    [out appendString:@"\n"];
}

char *splice_vision_analyze(const char *path, uint32_t kinds, double every, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVAssetReader *reader = nil;
        AVAssetReaderTrackOutput *output = video_output(url, &reader, NULL, NULL, error);
        if (output == nil) return NULL;
        NSMutableString *out = [NSMutableString string];
        VNFeaturePrintObservation *previous = nil;
        double next = 0;
        CMSampleBufferRef sample;
        while ((sample = [output copyNextSampleBuffer]) != NULL) {
            double t = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample));
            CVPixelBufferRef pixels = CMSampleBufferGetImageBuffer(sample);
            if (pixels == NULL || t + 1e-6 < next) {
                CFRelease(sample);
                continue;
            }
            next = t + every;
            VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCVPixelBuffer:pixels options:@{}];
            NSMutableArray<VNRequest *> *requests = [NSMutableArray array];
            VNGenerateImageFeaturePrintRequest *print = nil;
            VNRecognizeTextRequest *text = nil;
            VNDetectFaceRectanglesRequest *faces = nil;
            VNDetectHumanRectanglesRequest *people = nil;
            if (kinds & 1) [requests addObject:(print = [VNGenerateImageFeaturePrintRequest new])];
            if (kinds & 2) {
                text = [VNRecognizeTextRequest new];
                // ponytail: the fast recognizer reads UI text and loads in milliseconds; switch to
                // Accurate (seconds to load its model) if evals miss small or stylized text.
                text.recognitionLevel = VNRequestTextRecognitionLevelFast;
                text.usesLanguageCorrection = NO;
                [requests addObject:text];
            }
            if (kinds & 4) [requests addObject:(faces = [VNDetectFaceRectanglesRequest new])];
            if (kinds & 8) [requests addObject:(people = [VNDetectHumanRectanglesRequest new])];
            NSError *failure = nil;
            if (requests.count > 0 && ![handler performRequests:requests error:&failure]) {
                CFRelease(sample);
                splice_set_error(error, failure.localizedDescription.UTF8String ?: "Vision analysis failed.");
                return NULL;
            }
            VNFeaturePrintObservation *current = print.results.firstObject;
            if (current != nil) {
                float distance = 0;
                if (previous != nil) [current computeDistance:&distance toFeaturePrintObservation:previous error:nil];
                [out appendFormat:@"print\t%.4f\t%.4f\n", t, previous == nil ? 0.0 : distance];
                previous = current;
            }
            for (VNRecognizedTextObservation *line in text.results) {
                VNRecognizedText *best = [line topCandidates:1].firstObject;
                if (best != nil) box_line(out, "text", t, line.boundingBox, best.string);
            }
            for (VNFaceObservation *face in faces.results) box_line(out, "face", t, face.boundingBox, nil);
            for (VNHumanObservation *person in people.results) box_line(out, "person", t, person.boundingBox, nil);
            CFRelease(sample);
        }
        if (reader.status == AVAssetReaderStatusFailed) {
            splice_set_error(error, reader.error.localizedDescription.UTF8String ?: "Could not read the video.");
            return NULL;
        }
        return strdup(out.UTF8String);
    }
}

char *splice_vision_track(const char *path, double from, double to, double x, double y, double w, double h,
                          char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVAssetReader *reader = nil;
        AVAssetReaderTrackOutput *output = video_output(url, &reader, NULL, NULL, error);
        if (output == nil) return NULL;
        VNSequenceRequestHandler *sequence = [VNSequenceRequestHandler new];
        VNDetectedObjectObservation *object =
            [VNDetectedObjectObservation observationWithBoundingBox:CGRectMake(x, 1.0 - y - h, w, h)];
        NSMutableString *out = [NSMutableString string];
        CMSampleBufferRef sample;
        while ((sample = [output copyNextSampleBuffer]) != NULL) {
            double t = CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample));
            CVPixelBufferRef pixels = CMSampleBufferGetImageBuffer(sample);
            if (pixels == NULL || t + 1e-6 < from) {
                CFRelease(sample);
                continue;
            }
            if (t > to) {
                CFRelease(sample);
                break;
            }
            VNTrackObjectRequest *request = [[VNTrackObjectRequest alloc] initWithDetectedObjectObservation:object];
            request.trackingLevel = VNRequestTrackingLevelAccurate;
            NSError *failure = nil;
            if (![sequence performRequests:@[request] onCVPixelBuffer:pixels error:&failure]) {
                CFRelease(sample);
                splice_set_error(error, failure.localizedDescription.UTF8String ?: "Tracking failed.");
                return NULL;
            }
            VNDetectedObjectObservation *found = request.results.firstObject;
            if (found != nil && found.confidence > 0.2) object = found;
            box_line(out, "track", t, object.boundingBox, nil);
            CFRelease(sample);
        }
        return strdup(out.UTF8String);
    }
}

bool splice_vision_matte(const char *path, const char *output_path, char **error) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        AVAssetReader *reader = nil;
        double fps = 30;
        AVAssetReaderTrackOutput *output = video_output(url, &reader, &fps, NULL, error);
        if (output == nil) return false;
        NSURL *target = [NSURL fileURLWithFileSystemRepresentation:output_path isDirectory:NO relativeToURL:nil];
        [[NSFileManager defaultManager] removeItemAtURL:target error:nil];
        NSError *failure = nil;
        AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:target fileType:AVFileTypeQuickTimeMovie error:&failure];
        AVAssetWriterInput *input = nil;
        AVAssetWriterInputPixelBufferAdaptor *adaptor = nil;
        VNGeneratePersonSegmentationRequest *request = [VNGeneratePersonSegmentationRequest new];
        request.qualityLevel = VNGeneratePersonSegmentationRequestQualityLevelBalanced;
        request.outputPixelFormat = kCVPixelFormatType_OneComponent8;
        CIContext *context = [CIContext context];
        CMSampleBufferRef sample;
        bool started = false;
        while ((sample = [output copyNextSampleBuffer]) != NULL) {
            CVPixelBufferRef pixels = CMSampleBufferGetImageBuffer(sample);
            CMTime time = CMSampleBufferGetPresentationTimeStamp(sample);
            if (pixels == NULL) {
                CFRelease(sample);
                continue;
            }
            size_t width = CVPixelBufferGetWidth(pixels), height = CVPixelBufferGetHeight(pixels);
            if (!started) {
                input = [AVAssetWriterInput assetWriterInputWithMediaType:AVMediaTypeVideo outputSettings:@{
                    AVVideoCodecKey: AVVideoCodecTypeH264, AVVideoWidthKey: @(width), AVVideoHeightKey: @(height),
                }];
                input.expectsMediaDataInRealTime = NO;
                adaptor = [AVAssetWriterInputPixelBufferAdaptor assetWriterInputPixelBufferAdaptorWithAssetWriterInput:input
                    sourcePixelBufferAttributes:@{(NSString *)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
                                                  (NSString *)kCVPixelBufferWidthKey: @(width),
                                                  (NSString *)kCVPixelBufferHeightKey: @(height)}];
                if (writer == nil || ![writer canAddInput:input]) {
                    CFRelease(sample);
                    splice_set_error(error, "Could not create the matte movie.");
                    return false;
                }
                [writer addInput:input];
                [writer startWriting];
                [writer startSessionAtSourceTime:time];
                started = true;
            }
            VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithCVPixelBuffer:pixels options:@{}];
            if (![handler performRequests:@[request] error:&failure]) {
                CFRelease(sample);
                splice_set_error(error, failure.localizedDescription.UTF8String ?: "Segmentation failed.");
                return false;
            }
            CVPixelBufferRef mask = request.results.firstObject.pixelBuffer;
            CVPixelBufferRef frame = NULL;
            CVPixelBufferPoolCreatePixelBuffer(NULL, adaptor.pixelBufferPool, &frame);
            if (mask != NULL && frame != NULL) {
                CIImage *image = [CIImage imageWithCVPixelBuffer:mask];
                CGFloat sx = (CGFloat)width / CVPixelBufferGetWidth(mask), sy = (CGFloat)height / CVPixelBufferGetHeight(mask);
                image = [image imageByApplyingTransform:CGAffineTransformMakeScale(sx, sy)];
                [context render:image toCVPixelBuffer:frame];
                while (!input.readyForMoreMediaData) [NSThread sleepForTimeInterval:0.002];
                [adaptor appendPixelBuffer:frame withPresentationTime:time];
            }
            if (frame != NULL) CVPixelBufferRelease(frame);
            CFRelease(sample);
        }
        if (!started) {
            splice_set_error(error, "The video has no frames.");
            return false;
        }
        [input markAsFinished];
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        [writer finishWritingWithCompletionHandler:^{ dispatch_semaphore_signal(done); }];
        dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
        if (writer.status != AVAssetWriterStatusCompleted) {
            splice_set_error(error, writer.error.localizedDescription.UTF8String ?: "Could not finish the matte movie.");
            return false;
        }
        return true;
    }
}

#pragma clang diagnostic pop
