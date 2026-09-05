#version 330 core
out vec2 v_uv;
uniform vec2 u_surface_size;
uniform vec4 u_content_rect;

void main() {
    vec2 uvs[4] = vec2[4](
        vec2(0.0, 0.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(1.0, 1.0)
    );
    vec2 uv = uvs[gl_VertexID];
    vec2 pixel = u_content_rect.xy + uv * u_content_rect.zw;
    vec2 clip = vec2(
        pixel.x / u_surface_size.x * 2.0 - 1.0,
        1.0 - pixel.y / u_surface_size.y * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);
    v_uv = uv;
}
