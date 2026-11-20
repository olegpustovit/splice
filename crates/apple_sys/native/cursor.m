#include "platform.h"

#import <AppKit/AppKit.h>
#include <CommonCrypto/CommonDigest.h>
#include <math.h>
#include <string.h>

// The last cursor encoded on the application thread: its raw pixels, hotspot and hash. A poll
// runs many times a second and the cursor rarely changes, so an unchanged one is recognised
// from its pixels without re-encoding a PNG and hashing it (that was ~90 % of the capture
// agent's CPU over a long take).
static NSData *last_pixels;
static NSPoint last_hot;
static uint8_t last_hash[32];

static void *cursor_image(NSCursor *cursor, const uint8_t *previous, SpliceCursorInfo *out,
                          const uint8_t **data, size_t *length, char **error) {
    if (cursor == nil) return NULL;
    NSImage *image = cursor.image;
    NSBitmapImageRep *bitmap = nil;
    for (NSImageRep *representation in image.representations) {
        if ([representation isKindOfClass:[NSBitmapImageRep class]] &&
            representation.pixelsWide > bitmap.pixelsWide) {
            bitmap = (NSBitmapImageRep *)representation;
        }
    }
    if (bitmap == nil) {
        CGImageRef pixels = [image CGImageForProposedRect:NULL context:nil hints:nil];
        if (pixels != NULL) bitmap = [[NSBitmapImageRep alloc] initWithCGImage:pixels];
    }
    NSData *pixels = bitmap.bitmapData != NULL && !bitmap.isPlanar
        ? [NSData dataWithBytesNoCopy:bitmap.bitmapData
                               length:(NSUInteger)bitmap.bytesPerRow * (NSUInteger)bitmap.pixelsHigh
                         freeWhenDone:NO]
        : nil;
    if (previous != NULL && pixels != nil && last_pixels != nil &&
        memcmp(previous, last_hash, sizeof(last_hash)) == 0 && NSEqualPoints(cursor.hotSpot, last_hot) &&
        [pixels isEqualToData:last_pixels]) {
        memcpy(out->hash, last_hash, sizeof(last_hash));
        return NULL;
    }
    NSData *png = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    double scale = bitmap.pixelsWide / image.size.width;
    if (png.length == 0 || png.length > UINT32_MAX || !isfinite(scale) || scale <= 0.0) {
        splice_set_error(error, "Could not encode the system cursor.");
        return NULL;
    }
    const double metadata[] = {cursor.hotSpot.x * scale, cursor.hotSpot.y * scale, scale};
    CC_SHA256_CTX digest;
    CC_SHA256_Init(&digest);
    CC_SHA256_Update(&digest, metadata, (CC_LONG)sizeof(metadata));
    CC_SHA256_Update(&digest, png.bytes, (CC_LONG)png.length);
    CC_SHA256_Final(out->hash, &digest);
    last_pixels = pixels != nil ? [pixels copy] : nil;
    last_hot = cursor.hotSpot;
    memcpy(last_hash, out->hash, sizeof(last_hash));
    if (previous != NULL && memcmp(previous, out->hash, sizeof(out->hash)) == 0) return NULL;
    out->hot_x = metadata[0];
    out->hot_y = metadata[1];
    out->scale = metadata[2];
    *data = png.bytes;
    *length = png.length;
    return (__bridge_retained void *)png;
}

void *splice_cursor_current(const uint8_t *previous, SpliceCursorInfo *out,
                            const uint8_t **data, size_t *length, char **error) {
    if (![NSThread isMainThread]) {
        splice_set_error(error, "Read the system cursor from the application thread.");
        return NULL;
    }
    @autoreleasepool {
        // ponytail: use the public deprecated system-cursor accessor until AppKit replaces it.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        NSCursor *cursor = [NSCursor currentSystemCursor];
#pragma clang diagnostic pop
        return cursor_image(cursor, previous, out, data, length, error);
    }
}

// Uses a generated cursor image without reading or changing the active cursor.
bool splice_cursor_fixture(void) {
    @autoreleasepool {
        NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
            pixelsWide:16 pixelsHigh:16 bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES
            isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
        if (bitmap.bitmapData == NULL) return false;
        memset(bitmap.bitmapData, 255, (size_t)bitmap.bytesPerRow * 16);
        NSImage *image = [[NSImage alloc] initWithSize:NSMakeSize(8, 8)];
        [image addRepresentation:bitmap];
        image.size = NSMakeSize(8, 8);
        NSCursor *cursor = [[NSCursor alloc] initWithImage:image hotSpot:NSMakePoint(1.5, 2.5)];
        SpliceCursorInfo first = {0}, next = {0};
        const uint8_t *data = NULL;
        size_t length = 0;
        char *error = NULL;
        void *owner = cursor_image(cursor, NULL, &first, &data, &length, &error);
        const uint8_t signature[] = {0x89, 'P', 'N', 'G', 13, 10, 26, 10};
        bool valid = owner != NULL && length >= sizeof(signature) && memcmp(signature, data, sizeof(signature)) == 0 &&
            first.hot_x == 3.0 && first.hot_y == 5.0 && first.scale == 2.0;
        (void)(__bridge_transfer NSData *)owner;
        splice_string_free(error);
        error = NULL;
        owner = cursor_image(cursor, first.hash, &next, &data, &length, &error);
        bool unchanged = owner == NULL && error == NULL &&
            memcmp(first.hash, next.hash, sizeof(first.hash)) == 0;
        (void)(__bridge_transfer NSData *)owner;
        splice_string_free(error);
        return valid && unchanged;
    }
}
