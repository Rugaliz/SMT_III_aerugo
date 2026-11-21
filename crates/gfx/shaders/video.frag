#version 450
// Planar 4:2:0 YCbCr to RGB, as the original's video decoder shader does it (gamma color space),
// operation for operation and with the same constants. The result goes into a 10-bit render target.
layout(set = 0, binding = 0) uniform sampler2D y_plane;
layout(set = 0, binding = 1) uniform sampler2D cb_plane;
layout(set = 0, binding = 2) uniform sampler2D cr_plane;
layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 0) out vec4 o_color;

const float K_Y = uintBitsToFloat(0x3f940000u);      // 1.15625
const float K_CR_R = uintBitsToFloat(0x3fcc0000u);   // 1.59375
const float K_CB_G = uintBitsToFloat(0x3ec80000u);   // 0.390625
const float K_CR_G = uintBitsToFloat(0x3f500000u);   // 0.8125
const float K_CB_B = uintBitsToFloat(0x3ffe0000u);   // 1.984375
const float OFF_R = uintBitsToFloat(0xbf5f5ec8u);    // -0.87254
const float OFF_G = uintBitsToFloat(0x3f0807ddu);    // 0.53137
const float OFF_B = uintBitsToFloat(0xbf88c88au);    // -1.06862

void main() {
    float cb = texture(cb_plane, v_uv).r;
    vec2 t = cb * vec2(K_CB_G, K_CB_B);
    float y = texture(y_plane, v_uv).r;
    float g = y * K_Y - t.x;
    float b = y * K_Y + t.y;
    float cr = texture(cr_plane, v_uv).r;
    g = -cr * K_CR_G + g;
    float r = y * K_Y + cr * K_CR_R;
    o_color = vec4(r + OFF_R, g + OFF_G, b + OFF_B, 1.0);
}
