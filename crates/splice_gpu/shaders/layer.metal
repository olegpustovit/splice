// One quad per layer (PLAN.md T12.7, T16.3, T18). Straight-alpha output. Blending happens in the
// shader against the framebuffer (`[[color(0)]]`, Apple GPUs), so every blend mode shares one
// pipeline with hardware blending off.
#include <metal_stdlib>
using namespace metal;

struct Mask {
    float4 rect;     // x, y, w, h normalized to the layer's destination
    float4 params;   // shape (0 rect, 1 round, 2 ellipse), mode (0 blur, 1 pixelate, 2 cover, 3 spotlight), amount, feather
    float4 color;    // cover rgb, invert in w
};

// Mirrors splice_gpu::Layer (432 bytes in a 512-byte slot).
struct Layer {
    float4 dest;     // x, y, w, h in target pixels, top-left origin (before rotation)
    float4 crop;     // u, v, w, h normalized to the oriented source
    float4 color0;   // fill or ring color (straight alpha); shapes: fill
    float4 color1;   // gradient end color
    float2 target;   // target size in pixels
    float opacity;
    float corner;    // radius in pixels
    float shadow;    // 0..1
    float ring;      // ring width in pixels (kind 3); stroke width for shapes; texture: window inset width (color0)
    uint kind;       // 0 texture, 1 solid, 2 gradient, 3 ring, 4 ellipse, 5 line, 6 arrow, 7 callout, 8 box, 9 arc, 10 heart
    uint flags;      // 1 BGRA source, 2 BGRA output, 4 grade, 8 LUT, 16 key, 32 cutout, 64 cutout blur, 128 clip, 256 motion
    float2 pivot;    // rotation pivot in target pixels
    float rotation;  // radians, clockwise
    uint orient;     // source orientation: 0 none, 1 90°, 2 180°, 3 270°
    float4 grade0;   // exposure, contrast, saturation, vibrance
    float4 grade1;   // temperature, tint, highlights, shadows
    float4 lut;      // amount, cube size, cutout amount, unused
    float4 key0;     // key rgb, tolerance (shapes: stroke rgb, unused)
    float4 key1;     // softness, spill, unused, unused (ellipse and arc shapes: 1 - sweep; gradients: direction xy, angled flag z)
    float4 clip;     // visible region x, y, w, h in target pixels
    Mask masks[4];
    uint blend;      // 0 normal, 1 multiply, 2 screen, 3 overlay, 4 add, 5 darken, 6 lighten
    uint mask_count;
    uint pad0;
    uint pad1;
    float4 motion;   // flag 256: the crop these pixels showed one frame earlier (relative to dest)
};

struct Out {
    float4 position [[position]];
    float2 px;   // unrotated layer-space pixel
    float2 uv;
};

static float margin(constant Layer &l) {
    return l.shadow > 0.0 ? 0.08 * min(l.dest.z, l.dest.w) : 0.0;
}

vertex Out layer_vertex(uint id [[vertex_id]], constant Layer &l [[buffer(0)]]) {
    const float2 corners[6] = {float2(0, 0), float2(1, 0), float2(0, 1), float2(1, 0), float2(1, 1), float2(0, 1)};
    float m = margin(l) + (l.kind >= 4 ? l.ring : 0.0);
    float2 p = l.dest.xy - m + corners[id] * (l.dest.zw + 2.0 * m);
    float s = sin(l.rotation), c = cos(l.rotation);
    float2 d = p - l.pivot;
    float2 r = l.pivot + float2(d.x * c - d.y * s, d.x * s + d.y * c);
    Out o;
    o.px = p;
    o.uv = l.crop.xy + (p - l.dest.xy) / l.dest.zw * l.crop.zw;
    o.position = float4(r.x / l.target.x * 2.0 - 1.0, 1.0 - r.y / l.target.y * 2.0, 0.0, 1.0);
    return o;
}

static float rounded(float2 p, float2 c, float2 h, float r) {
    float2 q = abs(p - c) - h + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

static float ellipse(float2 p, float2 c, float2 h) {
    float2 q = (p - c) / max(h, float2(1e-3));
    return (length(q) - 1.0) * min(h.x, h.y);
}

static float segment(float2 p, float2 a, float2 b) {
    float2 pa = p - a, ba = b - a;
    float t = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * t);
}

// Oriented uv (as displayed) to decoded texture uv.
static float2 oriented(float2 uv, uint orient) {
    switch (orient) {
        case 1: return float2(uv.y, 1.0 - uv.x);
        case 2: return float2(1.0 - uv.x, 1.0 - uv.y);
        case 3: return float2(1.0 - uv.y, uv.x);
        default: return uv;
    }
}

static float4 fetch(texture2d<float> source, sampler s, float2 uv, constant Layer &l) {
    // A negative crop width or height samples mirrored (camera mirror, T46.3).
    float2 lo = min(l.crop.xy, l.crop.xy + l.crop.zw), hi = max(l.crop.xy, l.crop.xy + l.crop.zw);
    float4 c = source.sample(s, oriented(clamp(uv, lo, hi), l.orient));
    return (l.flags & 1u) ? c.bgra : c;
}

// Gradient progress 0…1: corner to corner, or along an angle (key1.xy, flag key1.z; T46.3).
static float gradient_t(float2 px, constant Layer &l) {
    if (l.key1.z > 0.5) return clamp(0.5 + dot(px - (l.dest.xy + l.dest.zw * 0.5), l.key1.xy), 0.0, 1.0);
    return clamp(dot(px - l.dest.xy, l.dest.zw) / dot(l.dest.zw, l.dest.zw), 0.0, 1.0);
}

static float luma(float3 c) { return dot(c, float3(0.2126, 0.7152, 0.0722)); }

static float3 grade(float3 c, constant Layer &l) {
    c *= exp2(l.grade0.x);
    c += float3(0.1, 0.0, -0.1) * l.grade1.x + float3(0.0, -0.1, 0.0) * l.grade1.y;
    float y = luma(c);
    c += l.grade1.z * 0.25 * smoothstep(0.5, 1.0, y) + l.grade1.w * 0.25 * (1.0 - smoothstep(0.0, 0.5, y));
    c = (c - 0.5) * l.grade0.y + 0.5;
    y = luma(c);
    float sat = max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
    float amount = l.grade0.z * (1.0 + l.grade0.w * (1.0 - sat));
    return clamp(mix(float3(y), c, amount), 0.0, 1.0);
}

// A cube of size n stored as n slices of n×n side by side (width n², height n).
static float3 lut3d(texture2d<float> table, sampler s, float3 c, float n) {
    float b = clamp(c.b, 0.0, 1.0) * (n - 1.0);
    float b0 = floor(b), b1 = min(b0 + 1.0, n - 1.0);
    float2 rg = (clamp(c.rg, 0.0, 1.0) * (n - 1.0) + 0.5) / float2(n * n, n);
    float3 a = table.sample(s, rg + float2(b0 / n, 0.0)).rgb;
    float3 z = table.sample(s, rg + float2(b1 / n, 0.0)).rgb;
    return mix(a, z, b - b0);
}

static float2 chroma(float3 c) {
    return float2(-0.1146 * c.r - 0.3854 * c.g + 0.5 * c.b, 0.5 * c.r - 0.4542 * c.g - 0.0458 * c.b);
}

// Golden-angle disk taps (unit radius) for the 16-tap blurs, precomputed (no trig per pixel).
constant float2 DISK[16] = {
    float2(0.176777, 0.000000), float2(-0.225772, 0.206827),
    float2(0.034556, -0.393771), float2(0.284575, 0.371170),
    float2(-0.522224, -0.092367), float2(0.494690, -0.314693),
    float2(-0.165454, 0.615528), float2(-0.315575, -0.607587),
    float2(0.684649, 0.250013), float2(-0.712248, 0.294030),
    float2(0.343331, -0.733740), float2(0.253759, 0.808923),
    float2(-0.764763, -0.443156), float2(0.897126, -0.197270),
    float2(-0.547472, 0.778797), float2(-0.126534, -0.976084)};

static float mask_cover(Mask m, float2 local, float2 size) {
    float2 h = m.rect.zw * 0.5 * size;
    float2 c = (m.rect.xy + m.rect.zw * 0.5) * size;
    float2 p = local * size;
    float d = m.params.x == 2.0 ? ellipse(p, c, h)
            : rounded(p, c, h, m.params.x == 1.0 ? 0.25 * min(h.x, h.y) : 0.0);
    float feather = max(m.params.w * min(size.x, size.y), 0.75);
    float inside = 1.0 - smoothstep(-feather, feather, d);
    return m.color.w > 0.5 ? 1.0 - inside : inside;
}

static float3 blend(float3 dst, float3 src, uint mode) {
    switch (mode) {
        case 1: return dst * src;
        case 2: return 1.0 - (1.0 - dst) * (1.0 - src);
        case 3: return select(1.0 - 2.0 * (1.0 - dst) * (1.0 - src), 2.0 * dst * src, dst < 0.5);
        case 4: return min(dst + src, 1.0);
        case 5: return min(dst, src);
        case 6: return max(dst, src);
        default: return src;
    }
}

// One body for every entry point; `full`, `hw` and `motion` are compile-time constants at each
// call site. `motion` true compiles the motion-blur taps (T46.2) into its own entry, so layers
// without blur never pay for them.
// `full` false compiles the fill entry (solid and gradient, kinds 1 and 2) without the picture,
// shape, mask, LUT and key paths. `hw` true leaves source-over to fixed-function blending (an
// opaque canvas and the normal blend mode): no framebuffer read, so overlapping layers do not
// serialize on programmable blending.
static inline float4 layer_body(Out in, constant Layer &l, texture2d<float> source, sampler linear,
                                texture2d<float> table, texture2d<float> matte, float4 framebuffer,
                                const bool full, const bool hw, const bool motion) {
    float4 dst = hw ? float4(0.0) : ((l.flags & 2u) ? framebuffer.bgra : framebuffer);
    float2 h = l.dest.zw * 0.5;
    float2 c = l.dest.xy + h;
    float2 local = (in.px - l.dest.xy) / l.dest.zw;
    float4 color;
    float coverage;
    if (!full) {
        float d = rounded(in.px, c, h, min(l.corner, min(h.x, h.y)));
        coverage = clamp(0.5 - d, 0.0, 1.0);
        if (l.kind == 1) {
            color = l.color0;
        } else {
            color = mix(l.color0, l.color1, gradient_t(in.px, l));
        }
    } else if (l.kind == 3) {
        float d = abs(length(in.px - c) - (h.x - l.ring * 0.5)) - l.ring * 0.5;
        coverage = clamp(0.5 - d, 0.0, 1.0);
        color = l.color0;
    } else if (l.kind >= 4) {
        // Shapes: fill (color0/color1 gradient) with an optional stroke (key0.rgb, ring px).
        float d;
        float2 a = l.dest.xy, b = l.dest.xy + l.dest.zw;
        if (l.kind == 4 || l.kind == 9) {
            d = ellipse(in.px, c, h);
            // A ring is the ellipse's band `ring` px thick.
            if (l.kind == 9) d = max(d, -(d + max(l.ring, 1.0)));
            // Progress (T45.4): only the first `sweep` of a turn, clockwise from 12 o'clock.
            float sweep = 1.0 - l.key1.x;
            if (sweep < 1.0) {
                float2 v = in.px - c;
                float a = atan2(v.x, -v.y);
                if (a < 0.0) a += 6.2831853;
                d = max(d, (a - sweep * 6.2831853) * length(v));
            }
        } else if (l.kind == 10) {
            // Heart (Inigo Quilez's distance), point down, fitted to the bounds.
            float2 p = float2(abs(in.px.x - c.x) / l.dest.z * 1.208,
                              (l.dest.y + l.dest.w - in.px.y) / l.dest.w * 1.104);
            float units = min(l.dest.z / 1.208, l.dest.w / 1.104);
            if (p.x + p.y > 1.0) {
                d = length(p - float2(0.25, 0.75)) - 0.35355339;
            } else {
                float2 q = p - 0.5 * max(p.x + p.y, 0.0);
                d = sqrt(min(dot(p - float2(0.0, 1.0), p - float2(0.0, 1.0)), dot(q, q))) * sign(p.x - p.y);
            }
            d *= units;
        } else if (l.kind == 5 || l.kind == 6) {
            d = segment(in.px, a, b) - max(l.ring, 1.0) * 0.5;
            if (l.kind == 6) {
                float2 dir = normalize(b - a);
                float2 n = float2(-dir.y, dir.x);
                float head = max(l.ring * 4.0, 12.0);
                float2 base = b - dir * head;
                d = min(d, min(segment(in.px, b, base + n * head * 0.6), segment(in.px, b, base - n * head * 0.6)) - max(l.ring, 1.0) * 0.5);
            }
        } else {
            d = rounded(in.px, c, h, min(l.corner, min(h.x, h.y)));
            if (l.kind == 7) {
                float2 tip = float2(c.x - h.x * 0.4, l.dest.y + l.dest.w + h.y * 0.6);
                d = min(d, segment(in.px, float2(c.x - h.x * 0.55, c.y + h.y * 0.8), tip) - h.y * 0.12);
            }
        }
        bool stroke_only = l.kind == 5 || l.kind == 6;
        float t = clamp(dot(in.px - l.dest.xy, l.dest.zw) / dot(l.dest.zw, l.dest.zw), 0.0, 1.0);
        float4 fill = stroke_only ? float4(l.key0.rgb, 1.0) : mix(l.color0, l.color1, t);
        float edge = (!stroke_only && l.ring > 0.0) ? clamp(0.5 - abs(d + l.ring * 0.5) + l.ring * 0.5, 0.0, 1.0) : 0.0;
        coverage = clamp(0.5 - d, 0.0, 1.0);
        color = float4(mix(fill.rgb, l.key0.rgb, edge), fill.a);
    } else {
        float d = rounded(in.px, c, h, min(l.corner, min(h.x, h.y)));
        coverage = clamp(0.5 - d, 0.0, 1.0);
        if (l.kind == 0) {
            if (motion && (l.flags & 256u)) {
                // Motion blur (T46.2): average 8 taps from this frame's uv to last frame's,
                // premultiplied; taps off the image (a cursor's trail) are transparent.
                float2 before = l.motion.xy + (in.px - l.dest.xy) / l.dest.zw * l.motion.zw;
                float4 sum = 0.0;
                for (int j = 0; j < 8; j++) {
                    float2 uv = mix(in.uv, before, float(j) / 7.0);
                    if (any(uv < 0.0) || any(uv > 1.0)) continue;
                    float4 c = fetch(source, linear, uv, l);
                    sum += float4(c.rgb * c.a, c.a);
                }
                color = sum.a > 0.0 ? float4(sum.rgb / sum.a, sum.a / 8.0) : float4(0.0);
            } else {
                color = fetch(source, linear, in.uv, l);
            }
            float m = 1.0;
            if (l.flags & 32u) {
                float2 lo = min(l.crop.xy, l.crop.xy + l.crop.zw), hi = max(l.crop.xy, l.crop.xy + l.crop.zw);
                m = matte.sample(linear, oriented(clamp(in.uv, lo, hi), l.orient)).r;
            }
            for (uint i = 0; i < l.mask_count; i++) {
                Mask k = l.masks[i];
                // Outside the mask's feathered box nothing changes (unless inverted): skip the
                // distance field for the common case of a small mask on a big picture.
                // The same feather mask_cover uses, in pixels, doubled for margin.
                float feather = max(k.params.w * min(l.dest.z, l.dest.w), 0.75);
                float2 slack = 2.0 * feather / l.dest.zw;
                if (k.color.w <= 0.5 && k.params.y != 3.0 &&
                    any(local < k.rect.xy - slack || local > k.rect.xy + k.rect.zw + slack)) continue;
                float inside = mask_cover(k, local, l.dest.zw);
                if (inside <= 0.0) continue;
                float amount = k.params.z;
                float4 treated;
                if (k.params.y == 0.0) {
                    // 16-tap golden-angle disk blur, radius up to 3% of the picture.
                    float radius = amount * 0.03;
                    // Per-pixel disk rotation (interleaved gradient noise): a fixed pattern leaves
                    // 16 ghost copies of sharp text; rotated, they average into fine grain.
                    float spin = 6.2831853 * fract(52.9829189 * fract(dot(in.px, float2(0.06711056, 0.00583715))));
                    float2x2 turn = float2x2(cos(spin), sin(spin), -sin(spin), cos(spin));
                    float4 sum = 0.0;
                    for (int j = 0; j < 16; j++) {
                        sum += fetch(source, linear, in.uv + (turn * DISK[j]) * radius * float2(1.0, l.dest.z / l.dest.w), l);
                    }
                    treated = sum / 16.0;
                } else if (k.params.y == 1.0) {
                    float cells = mix(120.0, 12.0, amount);
                    float2 cell = l.crop.zw / cells;
                    treated = fetch(source, linear, (floor(in.uv / cell) + 0.5) * cell, l);
                } else if (k.params.y == 2.0) {
                    treated = float4(k.color.rgb, 1.0);
                } else {
                    // Spotlight: darken outside the shape.
                    inside = 1.0 - inside;
                    treated = float4(color.rgb * (1.0 - 0.8 * amount), color.a);
                }
                color = mix(color, treated, inside);
            }
            if (l.flags & 4u) color.rgb = grade(color.rgb, l);
            if (l.flags & 8u) color.rgb = mix(color.rgb, lut3d(table, linear, color.rgb, l.lut.y), l.lut.x);
            if (l.flags & 16u) {
                float dist = distance(chroma(color.rgb), chroma(l.key0.rgb));
                color.a *= smoothstep(l.key0.w * 0.25, l.key0.w * 0.25 + max(l.key1.x * 0.25, 1e-3), dist);
                float3 k = l.key0.rgb;
                if (k.g >= k.r && k.g >= k.b) color.g = mix(color.g, min(color.g, max(color.r, color.b)), l.key1.y);
                else if (k.b >= k.r) color.b = mix(color.b, min(color.b, max(color.r, color.g)), l.key1.y);
            }
            if (l.flags & 32u) {
                if (l.flags & 64u) {
                    float4 sum = 0.0;
                    for (int j = 0; j < 16; j++) {
                        sum += fetch(source, linear, in.uv + DISK[j] * 0.02, l);
                    }
                    color = mix(color, mix(sum / 16.0, color, m), l.lut.z);
                } else {
                    color.a *= mix(1.0, m, l.lut.z);
                }
            }
            // Window inset (ring px, color0): an opaque inner border along the card's rounded edge.
            if (l.ring > 0.0) color = mix(color, float4(l.color0.rgb, 1.0), clamp(d + l.ring + 0.5, 0.0, 1.0));
        } else if (l.kind == 1) {
            color = l.color0;
        } else {
            color = mix(l.color0, l.color1, gradient_t(in.px, l));
        }
    }
    if (l.flags & 128u) {
        float2 p = in.position.xy;
        if (p.x < l.clip.x || p.y < l.clip.y || p.x >= l.clip.x + l.clip.z || p.y >= l.clip.y + l.clip.w) coverage = 0.0;
    }
    float alpha = color.a * coverage * l.opacity;
    float3 rgb = blend(dst.rgb, color.rgb, l.blend);
    float4 result = float4(rgb, alpha);
    if (l.shadow > 0.0 && l.kind < 3) {
        float blur = 0.03 * min(l.dest.z, l.dest.w);
        float d = rounded(in.px - float2(0.0, blur / 3.0), c, h, min(l.corner, min(h.x, h.y)));
        float shade = l.shadow * 0.55 * (1.0 - smoothstep(-blur, blur, d)) * l.opacity;
        float under = shade * (1.0 - alpha);
        float out_alpha = alpha + under;
        result = out_alpha > 0.0 ? float4(rgb * alpha / out_alpha, out_alpha) : float4(0.0);
    }
    if (hw) return (l.flags & 2u) ? result.bgra : result;
    // Source-over onto the framebuffer (straight alpha).
    float a = result.a + dst.a * (1.0 - result.a);
    float3 over = a > 0.0 ? (result.rgb * result.a + dst.rgb * dst.a * (1.0 - result.a)) / a : float3(0.0);
    float4 outc = float4(over, a);
    return (l.flags & 2u) ? outc.bgra : outc;
}

fragment float4 layer_fragment(Out in [[stage_in]], constant Layer &l [[buffer(0)]],
                               texture2d<float> source [[texture(1)]], sampler linear [[sampler(2)]],
                               texture2d<float> table [[texture(3)]], texture2d<float> matte [[texture(4)]],
                               float4 framebuffer [[color(0)]]) {
    return layer_body(in, l, source, linear, table, matte, framebuffer, true, false, false);
}

// Motion-blurred texture layers (flag 256), programmable blending like layer_fragment.
fragment float4 motion_fragment(Out in [[stage_in]], constant Layer &l [[buffer(0)]],
                                texture2d<float> source [[texture(1)]], sampler linear [[sampler(2)]],
                                texture2d<float> table [[texture(3)]], texture2d<float> matte [[texture(4)]],
                                float4 framebuffer [[color(0)]]) {
    return layer_body(in, l, source, linear, table, matte, framebuffer, true, false, true);
}

// Hardware-blended entries: they never read [[color(0)]].
fragment float4 layer_blend(Out in [[stage_in]], constant Layer &l [[buffer(0)]],
                            texture2d<float> source [[texture(1)]], sampler linear [[sampler(2)]],
                            texture2d<float> table [[texture(3)]], texture2d<float> matte [[texture(4)]]) {
    return layer_body(in, l, source, linear, table, matte, float4(0.0), true, true, false);
}

fragment float4 fill_blend(Out in [[stage_in]], constant Layer &l [[buffer(0)]],
                           texture2d<float> source [[texture(1)]], sampler linear [[sampler(2)]],
                           texture2d<float> table [[texture(3)]], texture2d<float> matte [[texture(4)]]) {
    return layer_body(in, l, source, linear, table, matte, float4(0.0), false, true, false);
}
