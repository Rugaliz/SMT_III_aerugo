#version 450
// One textured quad per draw, generated from gl_VertexIndex (triangle strip, 4 vertices).
layout(push_constant) uniform Push {
    vec4 pos01; // corners 0 and 1 (Vulkan NDC, y down)
    vec4 pos23; // corners 2 and 3
    vec4 uv01;
    vec4 uv23;
    vec4 color; // multiplier
} pc;
layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
void main() {
    int i = gl_VertexIndex & 3;
    vec4 p = i < 2 ? pc.pos01 : pc.pos23;
    vec4 t = i < 2 ? pc.uv01 : pc.uv23;
    bool odd = (i & 1) == 1;
    v_uv = odd ? t.zw : t.xy;
    v_color = pc.color;
    gl_Position = vec4(odd ? p.zw : p.xy, 0.0, 1.0);
}
