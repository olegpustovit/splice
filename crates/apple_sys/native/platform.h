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

// Cancels the attached project picker on the application thread.
void splice_app_cancel_open_panel(void);
bool splice_app_start_accessory(void);
bool splice_app_wait_accessory(double seconds);
void splice_app_alert(const char *title, const char *message);
char *splice_app_user_home(void);

typedef enum {
    SPLICE_PERMISSION_SCREEN,
    SPLICE_PERMISSION_MICROPHONE,
    SPLICE_PERMISSION_CAMERA,
    SPLICE_PERMISSION_INPUT,
} SplicePermission;

typedef enum {
    SPLICE_PERMISSION_NOT_GRANTED,
    SPLICE_PERMISSION_GRANTED,
    SPLICE_PERMISSION_NOT_DETERMINED,
    SPLICE_PERMISSION_DENIED,
    SPLICE_PERMISSION_RESTRICTED,
} SplicePermissionStatus;

uint32_t splice_permission_status(uint32_t permission);
bool splice_permission_request(uint32_t permission, bool open_settings);

typedef struct {
    double x, y, width, height;
} SpliceCaptureRect;

typedef struct {
    SpliceCaptureRect rect;
    double scale;
    uint64_t origin_ns, elapsed_ns;
    uint32_t kind;
    bool active;
} SpliceCaptureInputState;

typedef struct {
    double hot_x, hot_y, scale;
    uint8_t hash[32];
} SpliceCursorInfo;

_Static_assert(sizeof(SpliceCaptureInputState) == 64, "capture input state ABI");
_Static_assert(sizeof(SpliceCursorInfo) == 56, "cursor info ABI");

void *splice_cursor_current(const uint8_t *previous_hash, SpliceCursorInfo *out,
                            const uint8_t **data, size_t *length, char **error);

typedef enum {
    SPLICE_CAPTURE_DISPLAY,
    SPLICE_CAPTURE_WINDOW,
    SPLICE_CAPTURE_AREA,
    SPLICE_CAPTURE_APPLICATION,
} SpliceCaptureKind;

typedef struct {
    SpliceCaptureRect rect;
    uint32_t kind, id, main_display, owner_pid;
    char *app, *name;
} SpliceCaptureSource;

typedef struct {
    SpliceCaptureSource *items;
    size_t count;
} SpliceCaptureSources;

typedef struct {
    SpliceCaptureRect rect;
    uint32_t kind, id;
} SpliceCaptureTarget;

_Static_assert(sizeof(SpliceCaptureSource) == 64, "capture source ABI");
_Static_assert(sizeof(SpliceCaptureTarget) == 40, "capture target ABI");

bool splice_capture_sources(bool include_windows, SpliceCaptureSources *out, bool *denied, char **error);
void splice_capture_sources_dispose(SpliceCaptureSources *sources);
void *splice_capture_screenshot(const SpliceCaptureTarget *target, const uint8_t **data,
                                size_t *size, bool *denied, char **error);

typedef struct {
    double elapsed_seconds;
    uint64_t video_frames, dropped_frames;
} SpliceRecordingStats;

_Static_assert(sizeof(SpliceRecordingStats) == 24, "recording stats ABI");

void *splice_recording_start(const SpliceCaptureTarget *target, const char *directory, const char *microphone,
                             bool system_audio, bool camera, bool events, bool *denied, char **error);
bool splice_recording_status(void *recording, SpliceRecordingStats *out, bool *denied, char **error);
void splice_recording_pause(void *recording, bool pause);
bool splice_recording_finish(void *recording, SpliceRecordingStats *out, bool *denied, char **error);
void splice_recording_release(void *recording);
bool splice_recording_input_state(void *recording, SpliceCaptureInputState *out);

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

// Ramp values run from the start to the end of the instruction interval. `focus` is the canvas
// point the scale keeps fixed (x, y at the start, then at the end). `source` is the source clock
// in seconds, for cursor lookups. `corner` is a radius in canvas pixels; `shadow` is 0…1.
typedef struct {
    double crop[4], destination[4], scale[2], opacity[2], focus[4], source[2];
    double corner, shadow, cursor_scale;
    int32_t track;
    uint32_t center_crop;
    int32_t cursor;
    uint32_t flags;
} SpliceAVLayer;

enum { SpliceAVEase = 1, SpliceAVClickRings = 2 };

// One cursor state on the take's source clock. x and y are normalized to the captured source,
// top-left origin. `image` indexes the table's images, or -1 when hidden.
typedef struct {
    double t, x, y;
    int32_t image;
    uint32_t click;
} SpliceAVCursorSample;

typedef struct {
    const char *path;
    double hot[2], scale;
} SpliceAVCursorImage;

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
_Static_assert(sizeof(SpliceAVLayer) == 184 && offsetof(SpliceAVLayer, flags) == 180, "layer ABI");
_Static_assert(sizeof(SpliceAVCursorSample) == 32 && offsetof(SpliceAVCursorSample, click) == 28, "cursor ABI");
_Static_assert(sizeof(SpliceAVCursorImage) == 32 && offsetof(SpliceAVCursorImage, scale) == 24, "cursor image ABI");
_Static_assert(sizeof(SpliceAVGain) == 12 && offsetof(SpliceAVGain, gain) == 4, "gain ABI");
_Static_assert(sizeof(SpliceAVText) == 64 && offsetof(SpliceAVText, color) == 56, "text ABI");

void *splice_av_create(uint32_t width, uint32_t height, uint32_t fps, int32_t end, char **error);
void splice_av_release(void *composition);
int32_t splice_av_source(void *composition, const SpliceAVSource *source, char **error);
bool splice_av_range(void *composition, int32_t from, int32_t to,
                     const SpliceAVLayer *layers, size_t layer_count,
                     const SpliceAVGain *gains, size_t gain_count, char **error);
bool splice_av_text(void *composition, int32_t from, int32_t to, const SpliceAVText *text, char **error);
// Registers a take's cursor track. `points` is the source width in screen points. Returns its index.
int32_t splice_av_cursor(void *composition, const SpliceAVCursorSample *samples, size_t sample_count,
                         const SpliceAVCursorImage *images, size_t image_count, double points, char **error);
bool splice_av_finish(void *composition, char **error);
bool splice_av_read_frame(void *composition, int32_t frame, uint8_t *rgba, size_t length,
                         bool draw_text, char **error);
void *splice_av_player(void *composition, char **error);
void splice_av_player_play(void *player, bool playing);
bool splice_av_player_ended(void *player);
bool splice_av_player_seek(void *player, int32_t frame);
int32_t splice_av_player_copy(void *player, uint8_t *rgba, size_t length, int32_t *frame, char **error);
void *splice_av_texture(uint32_t width, uint32_t height, void **device, void **image, char **error);
int32_t splice_av_player_into_texture(void *player, void *texture, int32_t *frame, char **error);
void *splice_av_export(void *composition, const char *file, bool hevc, char **error);
int32_t splice_av_export_status(void *job, float *progress, char **error);
void splice_av_export_cancel(void *job);
void splice_av_export_release(void *job);

void *splice_image_png(const uint8_t *pixels, size_t length, uint32_t width, uint32_t height,
                       uint32_t max_edge, const uint8_t **data, size_t *size, char **error);
void splice_image_release(void *data);

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
size_t splice_recording_read_input(void *recording, SpliceEvent *out, size_t capacity);
uint64_t splice_recording_dropped_input(void *recording);

#endif
