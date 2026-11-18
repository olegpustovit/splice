#ifndef SPLICE_PLATFORM_H
#define SPLICE_PLATFORM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Fallible calls set an owned diagnostic on failure when one can be allocated.
// Call splice_error_free on any non-null diagnostic.
void splice_error_free(char *error);

// Internal helper for the shims. Accepts a null error output.
void splice_set_error(char **error, const char *message);

typedef struct {
    int32_t track_id;
    uint32_t codec;
    uint32_t channels;
} SpliceAudioTrack;

typedef struct {
    double duration_seconds;
    double nominal_fps;
    uint32_t width;
    uint32_t height;
    uint32_t video_codec;
    uint32_t video_tracks;
    size_t audio_count;
    SpliceAudioTrack *audio;
} SpliceMediaProbe;

_Static_assert(sizeof(SpliceAudioTrack) == 12 && _Alignof(SpliceAudioTrack) == 4, "audio track ABI");
_Static_assert(offsetof(SpliceAudioTrack, channels) == 8, "audio track layout");
_Static_assert(sizeof(SpliceMediaProbe) == 48 && _Alignof(SpliceMediaProbe) == 8, "media probe ABI");
_Static_assert(offsetof(SpliceMediaProbe, width) == 16, "video metadata layout");
_Static_assert(offsetof(SpliceMediaProbe, audio_count) == 32, "audio count layout");
_Static_assert(offsetof(SpliceMediaProbe, audio) == 40, "audio pointer layout");

// Writes one probe result for a local file. Dispose the result after reading it.
bool splice_media_probe(const char *path, SpliceMediaProbe *out, char **error);
void splice_media_dispose(SpliceMediaProbe *probe);

typedef struct {
    uint64_t time_ns;
    uint32_t kind;
    uint32_t flags;
    uint8_t payload[64];
} SpliceEvent;

_Static_assert(sizeof(SpliceEvent) == 80 && _Alignof(SpliceEvent) == 8, "event ABI");
_Static_assert(offsetof(SpliceEvent, kind) == 8, "event kind layout");
_Static_assert(offsetof(SpliceEvent, payload) == 16, "event payload layout");

typedef struct SpliceEventRing SpliceEventRing;
typedef void (*SpliceWake)(uint64_t token);

// The queue stores only the wake function pointer and its scalar token. Producers
// hold a live handle and may run on any native thread. There is one consumer.
SpliceEventRing *splice_events_create(size_t capacity, SpliceWake wake, uint64_t token, char **error);
void splice_events_release(SpliceEventRing *ring);
bool splice_events_push(SpliceEventRing *ring, const SpliceEvent *event);
size_t splice_events_peek(SpliceEventRing *ring, SpliceEvent *out, size_t capacity);
bool splice_events_ack(SpliceEventRing *ring, size_t count);
uint64_t splice_events_dropped(SpliceEventRing *ring);

#endif
