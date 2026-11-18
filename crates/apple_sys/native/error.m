#include "platform.h"

#include <stdlib.h>
#include <string.h>

void splice_set_error(char **error, const char *message) {
    if (error != NULL) {
        *error = message != NULL ? strdup(message) : NULL;
    }
}

void splice_string_free(char *value) {
    free(value);
}
