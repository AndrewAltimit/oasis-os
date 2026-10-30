#version 300 es
precision highp float;

// Gentle waves. Port of https://www.shadertoy.com/view/3fBBDc.
// GLSL twin of `software/waves.rs` (the CPU port every backend ships);
// keep the two in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // base sky colour (RGB 0-1)
uniform float u_speed;

out vec4 fragColor;

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    float time = u_time * u_speed;

    vec2 xy = fragCoord / u_resolution.xy;

    float edgeBlur   = 0.007;
    float waveDiff   = 100.0;
    float waveWidth  = 5.0;
    float waveHeight = 10.0;

    vec3 colDiff = vec3(0.1, 0.05, 0.1);

    vec3 col = u_color1.rgb - vec3(0.1, 0.15, 0.05) * xy.y;

    // First wave (cosine).
    float w1v = cos((xy.x + cos(time) / waveDiff + time * 0.05)
                    * waveWidth) / waveHeight;
    float w1 = w1v + 0.35;
    float w1b = w1v + 0.25;

    if (xy.y <= w1) {
        col += colDiff;
    } else if (abs(w1b - xy.y) < edgeBlur) {
        col += colDiff * (edgeBlur + (w1b - xy.y)) / edgeBlur;
    }

    // Second wave (sine).
    float w2v = sin((xy.x + sin(time) / waveDiff + time * 0.05)
                    * waveWidth) / waveHeight;
    float w2 = w2v + 0.5;
    float w2b = w2v + 0.25;

    if (xy.y <= w2) {
        col += colDiff;
    } else if (abs(w2b - xy.y) < edgeBlur) {
        col += colDiff * (edgeBlur + (w2b - xy.y)) / edgeBlur;
    }

    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
