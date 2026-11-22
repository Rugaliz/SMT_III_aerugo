#version 450
// Signed-distance-field text, vertex stage: the original's text shader with the underlay effect, line for
// line. Matrices are column arrays (c[0] is the first column).
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
    // face dilate, outline width, outline softness, weight normal
    vec4 p0;
    // weight bold, scale ratio A, scale ratio C, gradient scale
    vec4 p1;
    // underlay offset x, offset y, dilate, softness
    vec4 p2;
    // vertex offset x, y, texture width, height
    vec4 p3;
    // scale x, scale y, perspective filter, unused
    vec4 p4;
    // face uv speed x, y, outline uv speed x, y
    vec4 p5;
    // brightness, contrast, mask softness x, y
    vec4 p6;
} g;

layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec2 a_uv0;
layout(location = 4) in vec2 a_uv1;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_uv0;
layout(location = 2) out vec4 v_param;
layout(location = 3) out vec4 v_mask;
layout(location = 4) out vec3 v_view;
layout(location = 5) out vec4 v_underlay;
layout(location = 6) out vec4 v_underlay_color;
layout(location = 7) out vec4 v_tex;

void main() {
    float face_dilate = g.p0.x, outline_width = g.p0.y, outline_softness = g.p0.z, weight_normal = g.p0.w;
    float weight_bold = g.p1.x, ratio_a = g.p1.y, ratio_c = g.p1.z, gradient_scale = g.p1.w;
    mat4 o2w = g.object_to_world;
    mat4 w2o = g.world_to_object;
    mat4 vp = g.matrix_vp;
    mat4 proj = g.projection;

    vec2 pos = a_pos.xy + g.p3.xy;
    vec4 r1 = o2w[1] * pos.y;
    r1 = o2w[0] * pos.x + r1;
    r1 = o2w[2] * a_pos.z + r1;
    vec4 world = r1 + o2w[3];
    vec3 view = o2w[3].xyz * a_pos.w + r1.xyz;
    view = g.camera_pos.xyz - view;
    vec4 clip = vp[1] * world.y;
    clip = vp[0] * world.x + clip;
    clip = vp[2] * world.z + clip;
    clip = vp[3] * world.w + clip;
    gl_Position = clip;
    v_color = a_color;
    v_uv0 = a_uv0;

    vec3 n = vec3(dot(a_normal, w2o[0].xyz), dot(a_normal, w2o[1].xyz), dot(a_normal, w2o[2].xyz));
    n *= inversesqrt(dot(n, n));
    vec3 vdir = view * inversesqrt(dot(view, view));
    float nv = dot(n, vdir);
    vec2 px = proj[1].xy * g.screen_params.y;
    px = proj[0].xy * g.screen_params.x + px;
    px = abs(px) * g.p4.xy;
    px = clip.w / px;
    float px2 = dot(px, px);
    vec2 soft = g.p6.zw * 0.25 + px;
    v_mask.zw = vec2(0.25) / soft;
    float scale = inversesqrt(px2);
    scale = scale * (abs(a_uv1.y) * gradient_scale);
    float ortho_scale = scale * 1.5;
    float filt = abs(ortho_scale) * (1.0 - g.p4.z);
    float persp = scale * 1.5 - filt;
    persp = abs(nv) * persp + filt;
    float s = proj[3].w == 0.0 ? persp : ortho_scale;
    float bold = 0.0 >= a_uv1.y ? 1.0 : 0.0;
    float weight = bold * (weight_bold - weight_normal) + weight_normal;
    weight = weight * 0.25 + face_dilate;
    weight = weight * ratio_a;
    float half_weight = weight * 0.5;
    v_param.yw = vec2(s, half_weight);
    float bias_px = 0.5 / s;
    float clip_alpha = -outline_width * ratio_a + 1.0;
    clip_alpha = -outline_softness * ratio_a + clip_alpha;
    clip_alpha = clip_alpha * 0.5 - bias_px;
    v_param.x = -weight * 0.5 + clip_alpha;
    float bias = -weight * 0.5 + 0.5;
    v_param.z = bias_px + bias;

    vec4 cr = min(max(g.clip_rect, vec4(-2e10)), vec4(2e10));
    vec2 m = pos * 2.0 - cr.xy;
    v_mask.xy = m - cr.zw;
    vec3 e = g.env_matrix1.xyz * view.y;
    e = g.env_matrix0.xyz * view.x + e;
    v_view = g.env_matrix2.xyz * view.z + e;

    vec4 u = vec4(g.p2.w, g.p2.z, g.p2.x, g.p2.y) * ratio_c;
    float u_scale = u.x * s + 1.0;
    u_scale = s / u_scale;
    float u_bias = bias * u_scale - 0.5;
    float u_dilate = u_scale * u.y;
    vec2 u_off = -u.zw * gradient_scale;
    u_off = u_off / g.p3.zw;
    v_underlay.xy = u_off + a_uv0;
    v_underlay.z = u_scale;
    v_underlay.w = -u_dilate * 0.5 + u_bias;
    v_underlay_color.xyz = g.underlay_color.w * g.underlay_color.xyz;
    v_underlay_color.w = g.underlay_color.w;

    float packed = a_uv1.x * 0.000244140625;
    float hi = floor(packed);
    float lo = -hi * 4096.0 + a_uv1.x;
    vec2 tuv = vec2(hi, lo) * 0.001953125;
    v_tex.xy = tuv * g.face_tex_st.xy + g.face_tex_st.zw;
    v_tex.zw = tuv * g.outline_tex_st.xy + g.outline_tex_st.zw;
}
