#include "platform.h"

#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>

#include <math.h>

void *splice_image_png(const uint8_t *pixels, size_t length, uint32_t width, uint32_t height,
                       uint32_t max_edge, const uint8_t **data, size_t *size, char **error) {
    @autoreleasepool {
        const size_t stride = (size_t)width * 4;
        if (width == 0 || height == 0 || width > 16384 || height > 16384 || length != stride * height) {
            splice_set_error(error, "PNG encoding requires valid RGBA dimensions and a matching buffer.");
            return NULL;
        }
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGDataProviderRef provider = CGDataProviderCreateWithData(NULL, pixels, length, NULL);
        CGBitmapInfo layout = kCGBitmapByteOrder32Big | kCGImageAlphaPremultipliedLast;
        CGImageRef image = provider == NULL ? NULL :
            CGImageCreate(width, height, 8, 32, stride, color, layout, provider, NULL, true,
                          kCGRenderingIntentDefault);
        if (provider != NULL) CGDataProviderRelease(provider);
        if (image != NULL && max_edge > 0 && (width > max_edge || height > max_edge)) {
            const double factor = (double)max_edge / (width > height ? width : height);
            size_t w = (size_t)fmax(1, round(width * factor));
            size_t h = (size_t)fmax(1, round(height * factor));
            CGContextRef thumbnail = CGBitmapContextCreate(NULL, w, h, 8, 0, color, layout);
            CGImageRef scaled = NULL;
            if (thumbnail != NULL) {
                CGContextSetBlendMode(thumbnail, kCGBlendModeCopy);
                CGContextSetInterpolationQuality(thumbnail, kCGInterpolationHigh);
                CGContextDrawImage(thumbnail, CGRectMake(0, 0, w, h), image);
                scaled = CGBitmapContextCreateImage(thumbnail);
                CGContextRelease(thumbnail);
            }
            CGImageRelease(image);
            image = scaled;
        }
        CGColorSpaceRelease(color);
        if (image == NULL) {
            splice_set_error(error, "Could not create an image from the RGBA frame.");
            return NULL;
        }
        CFMutableDataRef bytes = CFDataCreateMutable(kCFAllocatorDefault, 0);
        CGImageDestinationRef destination = bytes == NULL ? NULL :
            CGImageDestinationCreateWithData(bytes, CFSTR("public.png"), 1, NULL);
        bool encoded = false;
        if (destination != NULL) {
            CGImageDestinationAddImage(destination, image, NULL);
            encoded = CGImageDestinationFinalize(destination);
            CFRelease(destination);
        }
        CGImageRelease(image);
        if (!encoded || CFDataGetLength(bytes) <= 0) {
            if (bytes != NULL) CFRelease(bytes);
            splice_set_error(error, "ImageIO could not encode the PNG.");
            return NULL;
        }
        *data = CFDataGetBytePtr(bytes);
        *size = (size_t)CFDataGetLength(bytes);
        return bytes;
    }
}

void splice_image_release(void *data) {
    CFRelease(data);
}

// Animated GIF (PLAN.md T21.1): `count` RGBA frames of width×height, `delay` seconds each,
// looping forever.
bool splice_image_gif(const char *path, const uint8_t *frames, size_t count, uint32_t width, uint32_t height,
                      double delay, char **error) {
    @autoreleasepool {
        if (count == 0 || width == 0 || height == 0) {
            splice_set_error(error, "A GIF needs frames.");
            return false;
        }
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        CGImageDestinationRef out = CGImageDestinationCreateWithURL((__bridge CFURLRef)url, CFSTR("com.compuserve.gif"),
                                                                   count, NULL);
        if (out == NULL) {
            splice_set_error(error, "Could not create the GIF.");
            return false;
        }
        NSDictionary *file = @{(id)kCGImagePropertyGIFDictionary: @{(id)kCGImagePropertyGIFLoopCount: @0}};
        CGImageDestinationSetProperties(out, (__bridge CFDictionaryRef)file);
        NSDictionary *frame = @{(id)kCGImagePropertyGIFDictionary: @{(id)kCGImagePropertyGIFDelayTime: @(delay)}};
        size_t stride = (size_t)width * 4;
        CGColorSpaceRef space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        for (size_t i = 0; i < count; i++) {
            CGDataProviderRef provider = CGDataProviderCreateWithData(NULL, frames + i * stride * height, stride * height, NULL);
            CGImageRef image = CGImageCreate(width, height, 8, 32, stride, space,
                                             kCGBitmapByteOrder32Big | kCGImageAlphaNoneSkipLast, provider, NULL,
                                             true, kCGRenderingIntentDefault);
            CGImageDestinationAddImage(out, image, (__bridge CFDictionaryRef)frame);
            CGImageRelease(image);
            CGDataProviderRelease(provider);
        }
        CGColorSpaceRelease(space);
        bool ok = CGImageDestinationFinalize(out);
        CFRelease(out);
        if (!ok) splice_set_error(error, "Could not write the GIF.");
        return ok;
    }
}
