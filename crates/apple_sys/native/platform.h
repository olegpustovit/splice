#ifndef SPLICE_PLATFORM_H
#define SPLICE_PLATFORM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Fallible calls set an owned diagnostic on failure when one can be allocated.
// Call splice_string_free on any non-null diagnostic or returned bookmark.
void splice_string_free(char *value);

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
char *splice_media_bookmark(const char *path, char **error);
void splice_media_dispose(SpliceMediaProbe *probe);

typedef struct {
    const char *path;
    double source_start, source_limit, speed, source_frame_duration;
    int32_t timeline[2];
    uint32_t kind, index, colors[2];
} SpliceAVSource;

typedef struct {
    double crop[4], destination[4], scale[2], opacity[2];
    int32_t track;
    uint32_t center_crop;
} SpliceAVLayer;

typedef struct {
    int32_t track;
    float gain[2];
} SpliceAVGain;

typedef struct {
    const char *text;
    double bounds[4], font;
    float opacity[2];
    uint32_t color;
} SpliceAVText;

_Static_assert(sizeof(SpliceAVSource) == 64 && offsetof(SpliceAVSource, timeline) == 40, "source ABI");
_Static_assert(sizeof(SpliceAVLayer) == 104 && offsetof(SpliceAVLayer, center_crop) == 100, "layer ABI");
_Static_assert(sizeof(SpliceAVGain) == 12 && offsetof(SpliceAVGain, gain) == 4, "gain ABI");
_Static_assert(sizeof(SpliceAVText) == 64 && offsetof(SpliceAVText, color) == 56, "text ABI");

void *splice_av_create(uint32_t width, uint32_t height, uint32_t fps, int32_t end, char **error);
void splice_av_release(void *composition);
int32_t splice_av_source(void *composition, const SpliceAVSource *source, char **error);
bool splice_av_range(void *composition, int32_t from, int32_t to,
                     const SpliceAVLayer *layers, size_t layer_count,
                     const SpliceAVGain *gains, size_t gain_count, char **error);
bool splice_av_text(void *composition, int32_t from, int32_t to, const SpliceAVText *text, char **error);
bool splice_av_finish(void *composition, char **error);
bool splice_av_read_frame(void *composition, int32_t frame, uint8_t *rgba, size_t length, char **error);
void *splice_av_export(void *composition, const char *file, bool hevc, char **error);
int32_t splice_av_export_status(void *job, float *progress, char **error);
void splice_av_export_cancel(void *job);
void splice_av_export_release(void *job);

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
