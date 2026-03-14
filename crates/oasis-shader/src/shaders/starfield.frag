#version 300 es
precision highp float;

// Animated starfield tunnel.
// Inspired by classic Shadertoy starfield effects.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // star colour (default white)
uniform vec4 u_color2; // background colour (default black)
uniform float u_speed;

out vec4 fragColor;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float time = u_time * speed;

    vec2 uv = (gl_FragCoord.xy - 0.5 * u_resolution.xy) / u_resolution.y;

    vec3 starCol = u_color1.a > 0.0 ? u_color1.rgb : vec3(1.0);
    vec3 bgCol   = u_color2.a > 0.0 ? u_color2.rgb : vec3(0.0, 0.0, 0.05);

    vec3 col = bgCol;

    // Multiple star layers at different depths.
    for (int layer = 0; layer < 4; layer++) {
        float depth = 1.0 + float(layer) * 0.5;
        float scale = 20.0 * depth;
        float fade = 1.0 / depth;

        vec2 st = uv * scale + vec2(time * 0.1 * depth, time * 0.05);
        vec2 cell = floor(st);
        vec2 f = fract(st) - 0.5;

        float h = hash(cell + float(layer) * 100.0);
        vec2 offset = vec2(h, hash(cell + 0.5)) - 0.5;
        float d = length(f - offset * 0.6);

        // Twinkle.
        float twinkle = 0.5 + 0.5 * sin(time * 3.0 + h * 6.28);
        float brightness = smoothstep(0.05, 0.0, d) * twinkle * fade;

        col += starCol * brightness * h;
    }

    fragColor = vec4(col, 1.0);
}
