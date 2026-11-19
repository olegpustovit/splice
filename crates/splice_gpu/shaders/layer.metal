// One quad per layer (PLAN.md T12.7). Straight-alpha output, blended source-over.
#include <metal_stdlib>
using namespace metal;

// Mirrors splice_gpu::Layer (96 bytes).
struct Layer {
    float4 dest;     // x, y, w, h in target pixels, top-left origin
    float4 crop;     // u, v, w, h normalized to the source texture
    float4 color0;   // fill or ring color (straight alpha)
    float4 color1;   // gradient end color
    float2 target;   // target size in pixels
    float opacity;
    float corner;    // radius in pixels
    float shadow;    // 0..1
    float ring;      // ring width in pixels (kind 3)
    uint kind;       // 0 texture, 1 solid, 2 gradient, 3 ring
    uint flags;      // 1: the texture holds BGRA bytes
};

struct Out {
    float4 position [[position]];
    float2 px;
    float2 uv;
};

// Room around the quad for the drop shadow.
static float margin(constant Layer &l) {
    return l.shadow > 0.0 ? 0.08 * min(l.dest.z, l.dest.w) : 0.0;
}

vertex Out layer_vertex(uint id [[vertex_id]], constant Layer &l [[buffer(0)]]) {
    const float2 corners[6] = {float2(0, 0), float2(1, 0), float2(0, 1), float2(1, 0), float2(1, 1), float2(0, 1)};
    float m = margin(l);
    float2 p = l.dest.xy - m + corners[id] * (l.dest.zw + 2.0 * m);
    Out o;
    o.px = p;
    o.uv = l.crop.xy + (p - l.dest.xy) / l.dest.zw * l.crop.zw;
    o.position = float4(p.x / l.target.x * 2.0 - 1.0, 1.0 - p.y / l.target.y * 2.0, 0.0, 1.0);
    return o;
}

// Signed distance to a rounded box centered at c with half size h.
static float rounded(float2 p, float2 c, float2 h, float r) {
    float2 q = abs(p - c) - h + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

fragment float4 layer_fragment(Out in [[stage_in]], constant Layer &l [[buffer(0)]],
                               texture2d<float> source [[texture(1)]], sampler linear [[sampler(2)]]) {
    float2 h = l.dest.zw * 0.5;
    float2 c = l.dest.xy + h;
    float4 color;
    float coverage;
    if (l.kind == 3) {
        float d = abs(length(in.px - c) - (h.x - l.ring * 0.5)) - l.ring * 0.5;
        coverage = clamp(0.5 - d, 0.0, 1.0);
        color = l.color0;
    } else {
        float d = rounded(in.px, c, h, min(l.corner, min(h.x, h.y)));
        coverage = clamp(0.5 - d, 0.0, 1.0);
        if (l.kind == 0) {
            color = source.sample(linear, clamp(in.uv, l.crop.xy, l.crop.xy + l.crop.zw));
            if (l.flags & 1u) color = color.bgra;
        } else if (l.kind == 1) {
            color = l.color0;
        } else {
            float t = clamp(dot(in.px - l.dest.xy, l.dest.zw) / dot(l.dest.zw, l.dest.zw), 0.0, 1.0);
            color = mix(l.color0, l.color1, t);
        }
    }
    float alpha = color.a * coverage * l.opacity;
    if (l.shadow > 0.0 && l.kind != 3) {
        float blur = 0.03 * min(l.dest.z, l.dest.w);
        float d = rounded(in.px - float2(0.0, blur / 3.0), c, h, min(l.corner, min(h.x, h.y)));
        float shade = l.shadow * 0.55 * (1.0 - smoothstep(-blur, blur, d)) * l.opacity;
        float under = shade * (1.0 - alpha);
        float out_alpha = alpha + under;
        return out_alpha > 0.0 ? float4(color.rgb * alpha / out_alpha, out_alpha) : float4(0.0);
    }
    return float4(color.rgb, alpha);
}
