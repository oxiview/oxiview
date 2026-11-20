// Coverage-weighted ordered compositing runs once at each pixel center.
// Each invocation owns one pixel and its opacity stack. Affine fallback uses
// sixteen subpixel inside tests; paint and compositing are not supersampled.
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
fn rounded_quadrant(q: vec2<f32>, radius: f32) -> f32 {
    let from_arc_center = q + vec2(radius);
    return length(max(from_arc_center, vec2(0.0)))
        + min(max(from_arc_center.x, from_arc_center.y), 0.0) - radius;
}
fn rounded_distance(i: u32, p: vec2<f32>) -> f32 {
    let local = vec2(scalar(i)*p.x + scalar(i+2u)*p.y + scalar(i+4u),
                     scalar(i+1u)*p.x + scalar(i+3u)*p.y + scalar(i+5u));
    let lo = point(i+6u);
    let hi = point(i+8u);
    let radii = color(i+10u);
    let half_size = (hi-lo)*0.5;
    if max(max(radii.x, radii.y), max(radii.z, radii.w)) > min(half_size.x, half_size.y) {
        // Large arcs can cross the box midlines. Intersect the four rounded
        // quadrants anchored at the actual corners, including their straight sides.
        let a = lo - local;
        let b = local - hi;
        return max(
            max(rounded_quadrant(a, radii.x), rounded_quadrant(vec2(b.x, a.y), radii.y)),
            max(rounded_quadrant(b, radii.z), rounded_quadrant(vec2(a.x, b.y), radii.w))
        );
    }
    let center = (lo+hi)*0.5;
    var radius = radii.x;
    if local.x > center.x {
        radius = select(radii.y, radii.z, local.y > center.y);
    } else if local.y > center.y { radius = radii.w; }
    return rounded_quadrant(abs(local-center) - half_size, radius);
}
fn coverage(offset: u32, p: vec2<f32>) -> f32 {
    let tag = words[offset];
    let i = offset + 1u;
    if tag == 0xffffffffu {
        // Convert local distance through the similarity scale for a one-pixel ramp.
        return clamp(0.5 - rounded_distance(i, p)*scalar(i+14u), 0.0, 1.0);
    }
    var hits = 0.0;
    // Fixed 4x4 pattern relative to the pixel center, confined to this shape.
    for (var sample = 0u; sample < 16u; sample++) {
        let delta = vec2(-0.375 + 0.25*f32(sample%4u), -0.375 + 0.25*f32(sample/4u));
        var hit: bool;
        if tag == 0xfffffffeu {
            hit = rounded_distance(i, p + delta) <= 0.0;
        } else {
            hit = inside(offset, p + delta);
        }
        hits += select(0.0, 1.0, hit);
    }
    return hits * 0.0625;
}
fn srgb(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3(0.0031308));
}
// Shared straight-sRGB image data lives once in the command-buffer atlas.
// Filter premultiplied colors so transparent pixels cannot introduce fringes.
fn paint(offset: u32, p: vec2<f32>) -> vec4<f32> {
    if words[offset] == 0xffffffffu {
        return color(offset + 1u);
    }
    let local = vec2(scalar(offset)*p.x + scalar(offset+2u)*p.y + scalar(offset+4u),
                     scalar(offset+1u)*p.x + scalar(offset+3u)*p.y + scalar(offset+5u));
    let start = point(offset+6u);
    let delta = point(offset+8u) - start;
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
    return value;
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
    let alpha = clamp(accumulated.a, 0.0, 1.0);
    let rgb = clamp(accumulated.rgb / max(alpha, 1e-8), vec3(0.0), vec3(1.0));
    textureStore(output, id.xy, vec4(srgb(rgb) * alpha, alpha));
}
