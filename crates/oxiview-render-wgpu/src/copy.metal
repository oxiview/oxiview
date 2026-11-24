#include <metal_stdlib>
using namespace metal;

// Mirrors copy.wgsl. Both paths consume the same wgpu scene texture.
vertex float4 vertex_copy(uint id [[vertex_id]]) {
    return float4(float2((id << 1u) & 2u, id & 2u) * 2.0 - 1.0, 0.0, 1.0);
}

static float3 decode_srgb(float3 c) {
    return select(pow((c + 0.055) / 1.055, float3(2.4)), c / 12.92, c <= 0.04045);
}

static float3 encode_srgb(float3 c) {
    return select(1.055 * pow(c, float3(1.0 / 2.4)) - 0.055, c * 12.92, c <= 0.0031308);
}

fragment float4 copy_unorm(float4 position [[position]], texture2d<float, access::read> source [[texture(0)]]) {
    return source.read(uint2(position.xy));
}

fragment float4 copy_srgb(float4 position [[position]], texture2d<float, access::read> source [[texture(0)]]) {
    const float4 c = source.read(uint2(position.xy));
    return float4(decode_srgb(c.rgb), c.a);
}

fragment float4 premultiplied_unorm(float4 position [[position]], texture2d<float, access::read> source [[texture(0)]]) {
    const float4 c = source.read(uint2(position.xy));
    return float4(encode_srgb(clamp(c.rgb / max(c.a, 1e-8), 0.0, 1.0)) * c.a, c.a);
}

fragment float4 premultiplied_srgb(float4 position [[position]], texture2d<float, access::read> source [[texture(0)]]) {
    const float4 c = source.read(uint2(position.xy));
    return float4(decode_srgb(encode_srgb(clamp(c.rgb / max(c.a, 1e-8), 0.0, 1.0)) * c.a), c.a);
}
