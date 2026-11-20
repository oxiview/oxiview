@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) id: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((id << 1u) & 2u), f32(id & 2u))*2.0-1.0, 0.0, 1.0);
}
fn load(position: vec4<f32>) -> vec4<f32> {
    return textureLoad(source, vec2<i32>(position.xy), 0);
}
fn decode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c+0.055)/1.055, vec3(2.4)), c/12.92, c <= vec3(0.04045));
}
fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0/2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
@fragment fn copy_unorm(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return load(position);
}
// The compute texture holds premultiplied sRGB bytes. Decode those bytes before
// the sRGB attachment encodes them, preserving offscreen output (including alpha).
@fragment fn srgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = load(position);
    return vec4(decode_srgb(c.rgb), c.a);
}
fn straight_srgb(c: vec4<f32>) -> vec4<f32> {
    return vec4(encode_srgb(clamp(c.rgb / max(c.a, 1e-8), vec3(0.0), vec3(1.0))), c.a);
}
@fragment fn straight_unorm(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return straight_srgb(load(position));
}
@fragment fn premultiplied_unorm(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = straight_srgb(load(position));
    return vec4(c.rgb * c.a, c.a);
}
@fragment fn premultiplied_srgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = straight_srgb(load(position));
    return vec4(decode_srgb(c.rgb * c.a), c.a);
}
