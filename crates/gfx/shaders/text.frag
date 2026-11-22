#version 450
// Signed-distance-field text, fragment stage: face, outline and underlay from the atlas distance, then
// the original's brightness and contrast adjustment.
layout(set = 0, binding = 0) uniform Globals {
    mat4 object_to_world;
    mat4 world_to_object;
    mat4 matrix_vp;
    mat4 projection;
    vec4 screen_params;
    vec4 camera_pos;
    vec4 time;
    vec4 face_color;
    vec4 outline_color;
    vec4 underlay_color;
    vec4 face_tex_st;
    vec4 outline_tex_st;
    vec4 clip_rect;
    vec4 env_matrix0;
    vec4 env_matrix1;
    vec4 env_matrix2;
    vec4 p0;
    vec4 p1;
    vec4 p2;
    vec4 p3;
    vec4 p4;
    vec4 p5;
    vec4 p6;
} g;
layout(set = 1, binding = 0) uniform sampler2D main_tex;
layout(set = 2, binding = 0) uniform sampler2D face_tex;
layout(set = 3, binding = 0) uniform sampler2D outline_tex;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv0;
layout(location = 2) in vec4 v_param;
layout(location = 3) in vec4 v_mask;
layout(location = 4) in vec3 v_view;
layout(location = 5) in vec4 v_underlay;
layout(location = 6) in vec4 v_underlay_color;
layout(location = 7) in vec4 v_tex;

layout(location = 0) out vec4 o_color;

void main() {
    float outline_width = g.p0.y, outline_softness = g.p0.z, ratio_a = g.p1.y;
    float bright = g.p6.x, contrast = g.p6.y;

    vec4 outline = texture(outline_tex, g.p5.zw * g.time.y + v_tex.zw) * g.outline_color;
    outline.xyz *= outline.w;
    vec3 face_rgb = v_color.xyz * g.face_color.xyz;
    vec4 face_s = texture(face_tex, g.p5.xy * g.time.y + v_tex.xy);
    face_rgb *= face_s.xyz;
    vec4 face = vec4(face_rgb, face_s.w * g.face_color.w);
    face.xyz = face_rgb * face.w;
    outline -= face;

    float ow = outline_width * ratio_a * v_param.y;
    float ow_mix = sqrt(min(ow, 1.0));
    ow *= 0.5;
    float sd = texture(main_tex, v_uv0).w;
    float d = -sd + v_param.z;
    float inner = clamp(d * v_param.y + ow, 0.0, 1.0);
    float outer = d * v_param.y - ow;
    vec4 c = (ow_mix * inner) * outline + face;
    float os = outline_softness * ratio_a;
    float os_px = os * v_param.y;
    float os_div = os * v_param.y + 1.0;
    outer = os_px * 0.5 + outer;
    outer = clamp(outer / os_div, 0.0, 1.0);
    outer = 1.0 - outer;
    vec4 c2 = c * outer;
    float rest = -c.w * outer + 1.0;

    float ud = texture(main_tex, v_underlay.xy).w;
    float ua = clamp(ud * v_underlay.z - v_underlay.w, 0.0, 1.0);
    c = (ua * v_underlay_color) * rest + c2;

    vec3 b = clamp(c.xyz * bright, 0.0, 1.0);
    vec3 sig = 1.0 / (exp2((b - 0.5) * -contrast * 1.442695022) + 1.0);
    bool any_rb = b.x != 0.0 || b.z != 0.0;
    sig = any_rb ? sig : vec3(0.0, b.y, 0.0);
    c.xyz = contrast != 0.0 ? sig : b;
    o_color = c * v_color.w;
}
