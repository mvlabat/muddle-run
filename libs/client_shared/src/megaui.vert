#version 450

layout(location = 0) in vec3 Vertex_Position;
layout(location = 1) in vec2 Vertex_Uv;
layout(location = 2) in vec4 Vertex_Color;

layout(location = 0) out vec2 v_Uv;
layout(location = 1) out vec4 v_Color;

layout(set = 0, binding = 0) uniform Transform {
    vec2 scale;
    vec2 translation;
};

void main() {
    v_Uv = Vertex_Uv;
    v_Color = Vertex_Color;
    gl_Position = vec4(Vertex_Position * scale + translation, 1.0);
}
