#include "platform.h"

#import <Foundation/Foundation.h>
#include <os/lock.h>
#include <stdlib.h>
#include <string.h>

@interface SPEventRing : NSObject {
@public
    os_unfair_lock lock;
    SpliceEvent *events;
    size_t capacity;
    size_t head;
    size_t count;
    uint64_t dropped;
    SpliceWake wake;
    uint64_t token;
}
@end

@implementation SPEventRing
- (void)dealloc {
    free(events);
}
@end

SpliceEventRing *splice_events_create(size_t capacity, SpliceWake wake, uint64_t token, char **error) {
    if (capacity == 0 || capacity > SIZE_MAX / sizeof(SpliceEvent)) {
        splice_set_error(error, "Invalid event ring capacity.");
        return NULL;
    }
    SPEventRing *ring = [SPEventRing new];
    if (ring == nil) {
        splice_set_error(error, "Could not allocate the native event ring.");
        return NULL;
    }
    ring->events = calloc(capacity, sizeof(SpliceEvent));
    if (ring->events == NULL) {
        splice_set_error(error, "Could not allocate event ring storage.");
        return NULL;
    }
    ring->lock = (os_unfair_lock)OS_UNFAIR_LOCK_INIT;
    ring->capacity = capacity;
    ring->wake = wake;
    ring->token = token;
    return (__bridge_retained SpliceEventRing *)ring;
}

void splice_events_release(SpliceEventRing *handle) {
    (void)(__bridge_transfer SPEventRing *)handle;
}

bool splice_events_push(SpliceEventRing *handle, const SpliceEvent *event) {
    SPEventRing *ring = (__bridge SPEventRing *)handle;
    os_unfair_lock_lock(&ring->lock);
    if (ring->count == ring->capacity) {
        if (ring->dropped < UINT64_MAX) {
            ring->dropped++;
        }
        os_unfair_lock_unlock(&ring->lock);
        return false;
    }
    bool wake = ring->count == 0 && ring->wake != NULL;
    size_t index = (ring->head + ring->count) % ring->capacity;
    ring->events[index] = *event;
    ring->count++;
    os_unfair_lock_unlock(&ring->lock);
    if (wake) {
        ring->wake(ring->token);
    }
    return true;
}

size_t splice_events_peek(SpliceEventRing *handle, SpliceEvent *out, size_t capacity) {
    SPEventRing *ring = (__bridge SPEventRing *)handle;
    os_unfair_lock_lock(&ring->lock);
    size_t count = ring->count < capacity ? ring->count : capacity;
    if (count != 0) {
        size_t first = ring->capacity - ring->head;
        if (first > count) {
            first = count;
        }
        memcpy(out, ring->events + ring->head, first * sizeof(SpliceEvent));
        if (first < count) {
            memcpy(out + first, ring->events, (count - first) * sizeof(SpliceEvent));
        }
    }
    os_unfair_lock_unlock(&ring->lock);
    return count;
}

bool splice_events_ack(SpliceEventRing *handle, size_t count) {
    SPEventRing *ring = (__bridge SPEventRing *)handle;
    os_unfair_lock_lock(&ring->lock);
    bool valid = count <= ring->count;
    if (valid) {
        ring->head = (ring->head + count) % ring->capacity;
        ring->count -= count;
    }
    os_unfair_lock_unlock(&ring->lock);
    return valid;
}

uint64_t splice_events_dropped(SpliceEventRing *handle) {
    SPEventRing *ring = (__bridge SPEventRing *)handle;
    os_unfair_lock_lock(&ring->lock);
    uint64_t dropped = ring->dropped;
    os_unfair_lock_unlock(&ring->lock);
    return dropped;
}
