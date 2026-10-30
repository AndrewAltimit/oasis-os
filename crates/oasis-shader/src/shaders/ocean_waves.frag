#version 300 es
precision highp float;

// Layered sine-wave ocean. Port of https://www.shadertoy.com/view/33t3WB.
// GLSL twin of `software/waves.rs` (the CPU port every backend ships);
// keep the two in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // background gradient, top row
uniform vec4 u_color2; // background gradient, bottom row
uniform vec4 u_color3; // wave colour
uniform float u_speed;

out vec4 fragColor;

#define PI 3.14159265359

float wave(float time, vec2 uv, vec4 amps, vec4 freqs, vec4 offset) {
    float x = uv.x;
    float y = 0.0;
    y += amps.x * sin(freqs.x * x + time + offset.x);
    y += amps.y * sin(freqs.y * x + time + offset.y);
    y += amps.z * sin(freqs.z * x + time + offset.z);
    y += amps.w * sin(freqs.w * x + time + offset.w);

    // smoothstep with edge0 > edge1 is undefined in GLSL: use 1 - s(b, a).
    float blur = 0.025;
    float top_wave = 1.0 - smoothstep(y, y + blur, uv.y);
    float bottom_wave = smoothstep(y - 1.0, y, uv.y) * 0.4;
    return top_wave * bottom_wave;
}

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    float time = u_time * u_speed;

    vec2 uv = 2.0 * (2.0 * fragCoord - u_resolution.xy) / u_resolution.y;

    vec3 col = mix(u_color1.rgb, u_color2.rgb, fragCoord.y / u_resolution.y);

    float f = 0.0;
    f += wave(time, uv,
              vec4(0.1, 0.2, 0.3, 0.4), vec4(0.1, 0.4, 0.8, 0.3),
              vec4(1.0, 1.5, 2.0, 2.5) * PI);
    f += wave(time, uv,
              vec4(0.1, 0.3, 0.4, 0.1), vec4(0.8, 0.5, 0.4, 0.3),
              vec4(5.0, 2.0, 1.0, 3.0));
    f += wave(time, uv,
              vec4(0.3, 0.2, 0.1, 0.2), vec4(0.9, 0.5, 0.1, 0.1),
              vec4(1.0, 2.0, 2.0, 3.0));

    fragColor = vec4(clamp(col + f * u_color3.rgb, 0.0, 1.0), 1.0);
}
