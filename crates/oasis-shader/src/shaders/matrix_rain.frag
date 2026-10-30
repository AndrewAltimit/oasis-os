#version 300 es
precision highp float;

// Matrix digital rain: falling character columns with a trailing fade.
// GLSL twin of `software/procedural.rs` (the CPU port every backend
// ships); keep the two in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // rain colour
uniform vec4 u_color2; // background colour
uniform float u_speed;

out vec4 fragColor;

// Same hash as the CPU renderer's `hash2`.
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    float time = u_time * u_speed;

    // Grid of "characters".
    float columns = 40.0;
    float cellW = u_resolution.x / columns;
    float cellH = cellW * 1.2;

    vec2 cell = floor(fragCoord / vec2(cellW, cellH));

    // Per-column speed and offset.
    float colHash = hash(vec2(cell.x, 0.0));
    float colSpeed = 1.0 + colHash * 3.0;
    float colOffset = colHash * 100.0;

    // Cells above the falling head (0 at the head); the head moves down.
    float fall = mod(time * colSpeed + colOffset - cell.y, 40.0);

    // Trail intensity -- bright at the head, fading behind it.
    // (Reversed-edge smoothsteps are undefined in GLSL: 1 - s(b, a).)
    float intensity = (1.0 - smoothstep(0.0, 20.0, fall))
                    * smoothstep(-1.0, 0.0, fall);

    // Flicker per character cell.
    float charHash = hash(cell + floor(time * 4.0));
    intensity *= 0.7 + 0.3 * charHash;

    // Bright head.
    float head = (1.0 - smoothstep(0.0, 1.0, fall)) * 2.0;

    vec3 col = u_color2.rgb + u_color1.rgb * intensity + vec3(head * 0.5);

    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
