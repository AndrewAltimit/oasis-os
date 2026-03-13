#version 300 es
precision highp float;

// Gentle blue waves.
// Port of https://www.shadertoy.com/view/3fBBDc

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // base sky colour (RGB 0-1)
uniform float u_speed;

out vec4 fragColor;

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float time = u_time * speed;

    vec2 xy = gl_FragCoord.xy / u_resolution.xy;

    float edgeBlur   = 0.007;
    float waveDiff   = 100.0;
    float waveWidth  = 5.0;
    float waveHeight = 10.0;

    // Default sky-blue (102, 204, 255) or user colour.
    vec3 bgCol = u_color1.a > 0.0 ? u_color1.rgb * 255.0
                                   : vec3(102.0, 204.0, 255.0);

    vec3 colDiff = vec3(0.1, 0.05, 0.1);

    vec3 col = vec3(bgCol.r / 255.0 - 0.1  * xy.y,
                    bgCol.g / 255.0 - 0.15 * xy.y,
                    bgCol.b / 255.0 - 0.05 * xy.y);

    // First wave (cosine).
    float w1 = cos((xy.x + cos(time) / waveDiff + time * 0.05)
                   * waveWidth) / waveHeight + 0.35;
    float w1b = w1 - 0.1; // +0.25 version

    if (xy.y <= w1) {
        col += colDiff;
    } else if (abs(w1b - xy.y) < edgeBlur) {
        col += colDiff * (edgeBlur + (w1b - xy.y)) * (1.0 / edgeBlur);
    }

    // Second wave (sine).
    float w2 = sin((xy.x + sin(time) / waveDiff + time * 0.05)
                   * waveWidth) / waveHeight + 0.5;
    float w2b = sin((xy.x + sin(time) / waveDiff + time * 0.05)
                    * waveWidth) / waveHeight + 0.25;

    if (xy.y <= w2) {
        col += colDiff;
    } else if (abs(w2b - xy.y) < edgeBlur) {
        col += colDiff * (edgeBlur + (w2b - xy.y)) * (1.0 / edgeBlur);
    }

    fragColor = vec4(col, 1.0);
}
