#version 300 es
precision highp float;

// Animated Voronoi cells. Port of https://www.shadertoy.com/view/WdlyRS.
// GLSL twin of `software/cells.rs` (the CPU port every backend ships);
// keep the two in step. The point hash is the CPU's single-sin hash
// (the original's cos/tan chain is ~3x dearer on the CPU).

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1;
uniform vec4 u_color2;
uniform float u_speed;
uniform float u_size;

out vec4 fragColor;

vec2 ran(vec2 p) {
    return fract(sin(vec2(dot(p, vec2(127.1, 311.7)),
                          dot(p, vec2(269.5, 183.3)))) * 43758.547);
}

vec2 pt(float t, vec2 id) {
    return sin(t * (ran(id + 0.5) - 0.5) + ran(id - 20.1) * 8.0) * 0.5;
}

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    // A non-positive size would divide by zero: use the default.
    float size = u_size > 0.0 ? u_size : 30.0;
    float t = u_time * u_speed * 2.0;

    vec2 uv = (fragCoord - 0.5 * u_resolution.xy) / u_resolution.x;
    vec2 off = u_time / vec2(50.0, 30.0);
    uv += off;
    uv *= size;

    vec2 gv = fract(uv) - 0.5;
    vec2 id = floor(uv);

    float mindist = 1e9;
    vec2 vorv = vec2(0.0);
    for (float i = -1.0; i <= 1.0; i++) {
        for (float j = -1.0; j <= 1.0; j++) {
            vec2 offv = vec2(i, j);
            vec2 p = pt(t, id + offv);
            float dist = length(gv + p - offv);
            if (dist < mindist) {
                mindist = dist;
                vorv = (id + p + offv) / size - off;
            }
        }
    }

    vec3 col = mix(u_color1.rgb, u_color2.rgb,
                   clamp(vorv.x * 2.2 + vorv.y, -1.0, 1.0) * 0.5 + 0.5);
    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
