#version 300 es
precision highp float;

// Multi-layer twinkling starfield. GLSL twin of `software/procedural.rs`
// (the CPU port every backend ships); keep the two in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // star colour
uniform vec4 u_color2; // background colour
uniform float u_speed;

out vec4 fragColor;

#define TAU 6.28318530718

// Same hash as the CPU renderer's `hash2`.
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    float time = u_time * u_speed;

    vec2 uv = (fragCoord - 0.5 * u_resolution.xy) / u_resolution.y;

    vec3 col = u_color2.rgb;

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

        // Twinkle. (smoothstep(0.05, 0.0, d) written with ordered edges:
        // edge0 > edge1 is undefined in GLSL.)
        float twinkle = 0.5 + 0.5 * sin(time * 3.0 + h * TAU);
        float brightness = (1.0 - smoothstep(0.0, 0.05, d)) * twinkle * fade;

        col += u_color1.rgb * brightness * h;
    }

    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
