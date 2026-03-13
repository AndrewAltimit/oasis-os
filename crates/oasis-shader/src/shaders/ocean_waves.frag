#version 300 es
precision highp float;

// Layered sine-wave ocean.
// Port of https://www.shadertoy.com/view/33t3WB

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // background bottom
uniform vec4 u_color2; // background top
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

    float blur = 0.025;
    float top_wave = smoothstep(y + blur, y, uv.y);
    float bottom_wave = smoothstep(y - 1.0, y, uv.y) * 0.4;
    return top_wave * bottom_wave;
}

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float time = u_time * speed;

    vec2 uv = 2.0 * (2.0 * gl_FragCoord.xy - u_resolution.xy) / u_resolution.y;

    vec3 bgBot = u_color1.a > 0.0 ? u_color1.rgb : vec3(0.0, 0.05, 0.4);
    vec3 bgTop = u_color2.a > 0.0 ? u_color2.rgb : vec3(0.0, 0.9, 0.9);
    vec3 waveCol = u_color3.a > 0.0 ? u_color3.rgb : vec3(0.6);

    fragColor.rgb = mix(bgBot, bgTop, gl_FragCoord.y / u_resolution.y);

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

    fragColor += vec4(f * waveCol, 1.0);
}
