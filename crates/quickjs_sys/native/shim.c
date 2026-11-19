// The only C that touches QuickJS-NG. No quickjs-libc: no std/os modules, files, sockets or timers.
#include <string.h>
#include "../vendor/quickjs.h"

typedef struct splice_qjs_host {
    void *opaque;
    // 0: `out` is result JSON; 1: `out` is JSON thrown as a value; 2: `out` is an uncatchable message.
    // `out` must be NUL-terminated (not counted in `out_len`) and live until the next call.
    int (*call)(void *opaque, const char *name, size_t name_len, const char *args,
                size_t args_len, const char **out, size_t *out_len);
    void (*log)(void *opaque, const char *text, size_t len);
    int (*interrupt)(void *opaque);
    // status 0: `json` is the result. status 1: `json` is the thrown value as JSON.
    void (*finish)(void *opaque, int status, const char *json, size_t json_len,
                   const char *message, size_t message_len, const char *stack, size_t stack_len);
} splice_qjs_host;

static int on_interrupt(JSContext *ctx, void *opaque) {
    (void)ctx;
    const splice_qjs_host *host = opaque;
    return host->interrupt(host->opaque);
}

static JSValue js_host(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    const splice_qjs_host *host = JS_GetContextOpaque(ctx);
    size_t name_len, args_len;
    const char *name = JS_ToCStringLen(ctx, &name_len, argv[0]);
    if (!name)
        return JS_EXCEPTION;
    const char *args = JS_ToCStringLen(ctx, &args_len, argv[1]);
    if (!args) {
        JS_FreeCString(ctx, name);
        return JS_EXCEPTION;
    }
    const char *out = "null";
    size_t out_len = 4;
    int status = host->call(host->opaque, name, name_len, args, args_len, &out, &out_len);
    JS_FreeCString(ctx, name);
    JS_FreeCString(ctx, args);
    if (status == 2) {
        JS_ThrowInternalError(ctx, "%.*s", (int)out_len, out);
        JSValue error = JS_GetException(ctx);
        JS_SetUncatchableError(ctx, error);
        return JS_Throw(ctx, error);
    }
    JSValue value = JS_ParseJSON(ctx, out, out_len, "<host>");
    if (status == 1 && !JS_IsException(value))
        return JS_Throw(ctx, value);
    return value;
}

static JSValue js_log(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    (void)argc;
    const splice_qjs_host *host = JS_GetContextOpaque(ctx);
    size_t len;
    const char *text = JS_ToCStringLen(ctx, &len, argv[0]);
    if (!text)
        return JS_EXCEPTION;
    host->log(host->opaque, text, len);
    JS_FreeCString(ctx, text);
    return JS_UNDEFINED;
}

// Converts with ToString; on failure (interrupt, memory) clears the exception and returns NULL.
static const char *to_text(JSContext *ctx, JSValueConst value, size_t *len) {
    const char *text = JS_ToCStringLen(ctx, len, value);
    if (!text)
        JS_FreeValue(ctx, JS_GetException(ctx));
    return text;
}

static void report(JSContext *ctx, const splice_qjs_host *host, JSValue value) {
    if (!JS_IsException(value)) {
        JSValue json = JS_JSONStringify(ctx, value, JS_UNDEFINED, JS_UNDEFINED);
        JS_FreeValue(ctx, value);
        if (!JS_IsException(json)) {
            size_t len = 4;
            const char *text = JS_IsUndefined(json) ? NULL : to_text(ctx, json, &len);
            JS_FreeValue(ctx, json);
            host->finish(host->opaque, 0, text ? text : "null", text ? len : 4, NULL, 0, NULL, 0);
            if (text)
                JS_FreeCString(ctx, text);
            return;
        }
    }
    JSValue error = JS_GetException(ctx);
    size_t message_len = 0, stack_len = 0, json_len = 0;
    const char *message = to_text(ctx, error, &message_len);
    const char *stack = NULL;
    if (JS_IsError(error)) {
        JSValue trace = JS_GetPropertyStr(ctx, error, "stack");
        if (JS_IsException(trace))
            JS_FreeValue(ctx, JS_GetException(ctx));
        else if (JS_IsString(trace))
            stack = to_text(ctx, trace, &stack_len);
        JS_FreeValue(ctx, trace);
    }
    const char *json = NULL;
    JSValue data = JS_JSONStringify(ctx, error, JS_UNDEFINED, JS_UNDEFINED);
    if (JS_IsException(data))
        JS_FreeValue(ctx, JS_GetException(ctx));
    else if (JS_IsString(data))
        json = to_text(ctx, data, &json_len);
    JS_FreeValue(ctx, data);
    JS_FreeValue(ctx, error);
    host->finish(host->opaque, 1, json ? json : "null", json ? json_len : 4,
                 message ? message : "exception", message ? message_len : 9, stack ? stack : "",
                 stack ? stack_len : 0);
    if (message)
        JS_FreeCString(ctx, message);
    if (stack)
        JS_FreeCString(ctx, stack);
    if (json)
        JS_FreeCString(ctx, json);
}

// Both sources must be NUL-terminated (not counted in their lengths). Returns -1 if the
// runtime could not be created; otherwise `finish` was called exactly once.
int splice_qjs_run(const char *prelude, size_t prelude_len, const char *source, size_t source_len,
                   size_t memory, size_t stack, const splice_qjs_host *host) {
    JSRuntime *rt = JS_NewRuntime();
    if (!rt)
        return -1;
    JS_SetMemoryLimit(rt, memory);
    JS_SetMaxStackSize(rt, stack);
    JS_SetInterruptHandler(rt, on_interrupt, (void *)host);
    JSContext *ctx = JS_NewContextRaw(rt);
    // No Performance (a clock) or WeakRef (GC-dependent): runs replay identically.
    if (!ctx || JS_AddIntrinsicBaseObjects(ctx) || JS_AddIntrinsicDate(ctx) ||
        JS_AddIntrinsicEval(ctx) || JS_AddIntrinsicRegExp(ctx) || JS_AddIntrinsicJSON(ctx) ||
        JS_AddIntrinsicProxy(ctx) || JS_AddIntrinsicMapSet(ctx) ||
        JS_AddIntrinsicTypedArrays(ctx) || JS_AddIntrinsicPromise(ctx) ||
        JS_AddIntrinsicBigInt(ctx)) {
        if (ctx)
            JS_FreeContext(ctx);
        JS_FreeRuntime(rt);
        return -1;
    }
    JS_SetContextOpaque(ctx, (void *)host);
    JSValue global = JS_GetGlobalObject(ctx);
    JS_SetPropertyStr(ctx, global, "__host", JS_NewCFunction(ctx, js_host, "__host", 2));
    JS_SetPropertyStr(ctx, global, "__log", JS_NewCFunction(ctx, js_log, "__log", 1));
    JS_FreeValue(ctx, global);
    JSValue value = JS_Eval(ctx, prelude, prelude_len, "<prelude>", JS_EVAL_TYPE_GLOBAL);
    if (!JS_IsException(value)) {
        JS_FreeValue(ctx, value);
        value = JS_Eval(ctx, source, source_len, "code.js", JS_EVAL_TYPE_GLOBAL);
    }
    // Headroom so the result and error text can still be read after a memory error.
    JS_SetMemoryLimit(rt, memory + memory / 4);
    report(ctx, host, value);
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
    return 0;
}
