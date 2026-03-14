#version 300 es
precision highp float;

// Classic plasma effect.
// Based on the demoscene plasma technique using overlapping sine waves.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // palette colour 1 (default magenta)
uniform vec4 u_color2; // palette colour 2 (default cyan)
uniform vec4 u_color3; // palette colour 3 (default yellow)
uniform float u_speed;

out vec4 fragColor;

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float time = u_time * speed;

    vec2 uv = gl_FragCoord.xy / u_resolution.xy;

    // Four overlapping sine waves.
    float v1 = sin(uv.x * 10.0 + time);
    float v2 = sin(10.0 * (uv.x * sin(time * 0.5)
                          + uv.y * cos(time * 0.3)) + time);
    float v3 = sin(sqrt(100.0 * ((uv.x - 0.5) * (uv.x - 0.5)
                                + (uv.y - 0.5) * (uv.y - 0.5)) + 1.0) + time);
    float v4 = sin(sqrt(100.0 * ((uv.x - 0.3) * (uv.x - 0.3)
                                + (uv.y - 0.7) * (uv.y - 0.7)) + 1.0)
               + time * 0.7);

    float v = (v1 + v2 + v3 + v4) * 0.25;

    // Map to palette colours.
    vec3 c1 = u_color1.a > 0.0 ? u_color1.rgb : vec3(1.0, 0.0, 0.5);
    vec3 c2 = u_color2.a > 0.0 ? u_color2.rgb : vec3(0.0, 0.8, 1.0);
    vec3 c3 = u_color3.a > 0.0 ? u_color3.rgb : vec3(1.0, 0.9, 0.0);

    float t = v * 0.5 + 0.5; // normalize to 0..1
    vec3 col;
    if (t < 0.33) {
        col = mix(c1, c2, t * 3.0);
    } else if (t < 0.66) {
        col = mix(c2, c3, (t - 0.33) * 3.0);
    } else {
        col = mix(c3, c1, (t - 0.66) * 3.0);
    }

    fragColor = vec4(col, 1.0);
}
