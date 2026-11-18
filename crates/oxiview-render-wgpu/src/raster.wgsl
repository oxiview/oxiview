// Coverage-weighted ordered compositing runs once at each pixel center.
// Each invocation owns one pixel and its opacity stack. Only polygon coverage
// uses sixteen subpixel inside tests; paint and compositing are not supersampled.
// A matched filled box and inside border share one outer/inner coverage result
// so their common antialiased boundary is not composited twice.
@group(0) @binding(0) var<storage, read> words: array<u32>;
@group(0) @binding(1) var<storage, read_write> layers: array<vec4<f32>>;
@group(0) @binding(2) var output: texture_storage_2d<rgba8unorm, write>;
fn scalar(i: u32) -> f32 { return bitcast<f32>(words[i]); }
fn point(i: u32) -> vec2<f32> { return vec2(scalar(i), scalar(i + 1u)); }
fn color(i: u32) -> vec4<f32> { return vec4(scalar(i), scalar(i+1u), scalar(i+2u), scalar(i+3u)); }
fn inside(offset: u32, p: vec2<f32>) -> bool {
    let n = words[offset];
    if n < 3u { return false; }
    var hit = false;
    var a = point(offset + 1u + 2u * (n - 1u));
    for (var j = 0u; j < n; j++) {
        let b = point(offset + 1u + 2u * j);
        if (a.y > p.y) != (b.y > p.y) {
            if p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x { hit = !hit; }
        }
        a = b;
    }
    return hit;
}
// SDF draws and rounded-rectangle clips evaluate distance once at the pixel
// center, convert local distance to physical pixels with the similarity scale,
// and apply a one-pixel linear ramp: clamp(0.5 - distance_px, 0, 1).
fn coverage(offset: u32, p: vec2<f32>) -> f32 {
    if words[offset] != 0xffffffffu {
        var hits = 0.0;
        // Fixed 4x4 pattern relative to the pixel center, confined to this shape.
        for (var sample = 0u; sample < 16u; sample++) {
            let delta = vec2(-0.375 + 0.25*f32(sample%4u), -0.375 + 0.25*f32(sample/4u));
            hits += select(0.0, 1.0, inside(offset, p + delta));
        }
        return hits * 0.0625;
    }
    let i = offset + 1u;
    let local = vec2(scalar(i)*p.x + scalar(i+2u)*p.y + scalar(i+4u),
                     scalar(i+1u)*p.x + scalar(i+3u)*p.y + scalar(i+5u));
    let lo = point(i+6u);
    let hi = point(i+8u);
    let center = (lo+hi)*0.5;
    var radius = scalar(i+10u);
    if local.x > center.x {
        radius = select(scalar(i+11u), scalar(i+12u), local.y > center.y);
    } else if local.y > center.y { radius = scalar(i+13u); }
    let q = abs(local-center) - (hi-lo)*0.5 + vec2(radius);
    let distance_px = (length(max(q, vec2(0.0))) + min(max(q.x,q.y),0.0) - radius)*scalar(i+14u);
    return clamp(0.5 - distance_px, 0.0, 1.0);
}
fn srgb(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3(0.0031308));
}
// Shared straight-sRGB image data lives once in the command-buffer atlas.
// Filter premultiplied colors so transparent pixels cannot introduce fringes.
fn image_pixel(offset: u32, size: vec2<u32>, p: vec2<i32>) -> vec4<f32> {
    let xy = vec2<u32>(clamp(p, vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1)));
    let packed = words[offset + xy.y * size.x + xy.x];
    let value = vec4<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u),
                          f32((packed >> 16u) & 255u), f32(packed >> 24u)) / 255.0;
    return vec4(value.rgb * value.a, value.a);
}
fn paint(offset: u32, p: vec2<f32>) -> vec4<f32> {
    if words[offset] == 0xffffffffu {
        return color(offset + 1u);
    }
    let local = vec2(scalar(offset)*p.x + scalar(offset+2u)*p.y + scalar(offset+4u),
                     scalar(offset+1u)*p.x + scalar(offset+3u)*p.y + scalar(offset+5u));
    let start = point(offset+6u);
    let delta = point(offset+8u) - start;
    if words[offset+10u] == 0xffffffffu {
        let size = vec2<u32>(words[offset+11u], words[offset+12u]);
        let data = words[offset+13u];
        let uv = (local - start) / delta * vec2<f32>(size) - vec2(0.5);
        let base = vec2<i32>(floor(uv));
        let weight = fract(uv);
        let a = mix(image_pixel(data,size,base), image_pixel(data,size,base+vec2<i32>(1,0)), weight.x);
        let b = mix(image_pixel(data,size,base+vec2<i32>(0,1)), image_pixel(data,size,base+vec2<i32>(1,1)), weight.x);
        return mix(a,b,weight.y);
    }
    let length2 = dot(delta, delta);
    var t = 0.0;
    if length2 > 0.0 { t = dot(local-start, delta) / length2; }
    let n = words[offset+10u];
    var stop = offset+11u;
    var value = color(stop+1u);
    // Equal stop positions choose the last stop at the transition.
    for (var j = 1u; j < n; j++) {
        let next = stop + 5u;
        if t < scalar(next) {
            let span = scalar(next)-scalar(stop);
            if span > 0.0 { value = mix(color(stop+1u), color(next+1u), clamp((t-scalar(stop))/span, 0.0, 1.0)); }
            break;
        }
        stop = next;
        value = color(stop+1u);
    }
    if value.a <= 0.0 { return vec4(0.0); }
    let alpha = clamp(value.a, 0.0, 1.0);
    return vec4(clamp(srgb(value.rgb/value.a), vec3(0.0), vec3(1.0)) * alpha, alpha);
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= words[0] || id.y >= words[1] { return; }
    let base = (id.y * words[0] + id.x) * words[2];
    let p = vec2<f32>(id.xy) + vec2(0.5);
    var accumulated = vec4(0.0);
    var depth = 0u;
    let tile = (id.y / 16u) * ((words[0] + 15u) / 16u) + id.x / 16u;
    let list = words[words[3] + tile * 2u];
    let count = words[words[3] + tile * 2u + 1u];
    for (var index = 0u; index < count; index++) {
        let command = words[list + index];
        let op = words[command];
        if op == 1u {
            layers[base+depth] = accumulated;
            depth++;
            accumulated = vec4(0.0);
        } else if op == 2u {
            depth--;
            accumulated *= scalar(command+2u);
            accumulated += layers[base+depth] * (1.0-accumulated.a);
        } else if op == 3u {
            var outer = coverage(words[command+2u], p);
            var inner = min(outer, coverage(words[command+3u], p));
            for (var clip = 0u; clip < words[command+4u] && outer > 0.0; clip++) {
                let covered = coverage(words[command+7u+clip], p);
                outer = min(outer, covered);
                inner = min(inner, covered);
            }
            if outer > 0.0 {
                let background = paint(words[command+5u], p);
                let border = paint(words[command+6u], p);
                let border_over_background = border + background * (1.0-border.a);
                let source = background * inner + border_over_background * (outer-inner);
                accumulated = source + accumulated*(1.0-source.a);
            }
        } else {
            var covered = max(coverage(words[command+2u], p) - coverage(words[command+3u], p), 0.0);
            for (var clip = 0u; clip < words[command+4u] && covered > 0.0; clip++) {
                covered = min(covered, coverage(words[command+6u+clip], p));
            }
            if covered > 0.0 {
                let source = paint(words[command+5u], p) * covered;
                accumulated = source + accumulated*(1.0-source.a);
            }
        }
    }
    textureStore(output, id.xy, accumulated);
}
