#pragma once
#include <math.h>
#include <stddef.h>

typedef struct SpliceCaptionWord {
    double advance;
    double scale;
    double reserve;
    double slot_x;
    double row_width;
    double x;
    double y;
} SpliceCaptionWord;

// Slot widths remain fixed while words are hidden or animated.
static void splice_caption_place(SpliceCaptionWord *words, size_t count, double font, double space,
                                 const double box[5]) {
    const double gap = fmax(space, font * 0.3);
    const double line_height = font * 1.2;
    size_t first = 0, row = 0;
    while (first < count) {
        size_t end = first;
        double width = 0.0;
        while (end < count) {
            const double reserve = words[end].advance * words[end].reserve;
            const double next = width + (end == first ? 0.0 : gap) + reserve;
            if (end != first && next > box[4]) break;
            words[end].x = reserve;
            width = next;
            end++;
        }
        double x = box[0] - width * 0.5;
        for (size_t i = first; i < end; i++) {
            const double reserve = words[i].x;
            words[i].slot_x = x;
            words[i].row_width = i == first ? width : 0;
            words[i].x = x + (reserve - words[i].advance * words[i].scale) * 0.5;
            words[i].y = (double)row * line_height;
            x += reserve + gap;
        }
        first = end;
        row++;
    }
    const double half = (double)row * line_height * 0.5;
    const double center = fmin(fmax(box[1], box[2] + half), box[3] - half);
    for (size_t i = 0; i < count; i++) words[i].y += center - half + line_height * 0.5;
}
