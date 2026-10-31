#version 300 es
precision highp float;

// Classic demoscene plasma: overlapping sine waves mapped onto a
// three-colour palette. GLSL twin of `software/procedural.rs` (the CPU
// port every backend ships); keep the two in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // palette colour 1
uniform vec4 u_color2; // palette colour 2
uniform vec4 u_color3; // palette colour 3
uniform float u_speed;

out vec4 fragColor;

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    float time = u_time * u_speed;

    vec2 uv = fragCoord / u_resolution.xy;

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

    vec3 c1 = u_color1.rgb;
    vec3 c2 = u_color2.rgb;
    vec3 c3 = u_color3.rgb;

    float t = v * 0.5 + 0.5; // normalize to 0..1
    vec3 col;
    if (t < 0.33) {
        col = mix(c1, c2, t * 3.0);
    } else if (t < 0.66) {
        col = mix(c2, c3, (t - 0.33) * 3.0);
    } else {
        col = mix(c3, c1, (t - 0.66) * 3.0);
    }

    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
