@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) id: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((id << 1u) & 2u), f32(id & 2u))*2.0-1.0, 0.0, 1.0);
}
fn load(position: vec4<f32>) -> vec4<f32> {
    return textureLoad(source, vec2<i32>(position.xy), 0);
}
@fragment fn copy_unorm(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return load(position);
}
// The compute texture holds premultiplied sRGB bytes. Decode those bytes before
// the sRGB attachment encodes them, preserving offscreen output (including alpha).
@fragment fn srgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = load(position);
    let linear = select(pow((c.rgb+0.055)/1.055, vec3(2.4)), c.rgb/12.92, c.rgb <= vec3(0.04045));
    return vec4(linear, c.a);
}
