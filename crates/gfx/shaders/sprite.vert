#version 450
// One textured quad per draw, generated from gl_VertexIndex (triangle strip, 4 vertices).
layout(push_constant) uniform Push {
    vec4 dst;   // NDC rect: x0, y0, x1, y1 (Vulkan NDC, y down)
    vec4 uv;    // u0, v0, u1, v1
    vec4 color; // multiplier
} pc;
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
void main() {
    vec2 c = vec2(float(gl_VertexIndex & 1), float((gl_VertexIndex >> 1) & 1));
    v_uv = mix(pc.uv.xy, pc.uv.zw, c);
    v_color = pc.color;
    gl_Position = vec4(mix(pc.dst.xy, pc.dst.zw, c), 0.0, 1.0);
}
