#version 300 es
precision highp float;

// Matrix digital rain effect.
// Falling character columns with trailing fade.

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1; // rain colour (default green)
uniform vec4 u_color2; // background colour (default black)
uniform float u_speed;

out vec4 fragColor;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float time = u_time * speed;

    vec3 rainCol = u_color1.a > 0.0 ? u_color1.rgb : vec3(0.0, 1.0, 0.3);
    vec3 bgCol   = u_color2.a > 0.0 ? u_color2.rgb : vec3(0.0, 0.02, 0.0);

    // Grid of "characters".
    float columns = 40.0;
    float cellW = u_resolution.x / columns;
    float cellH = cellW * 1.2;

    vec2 cell = floor(gl_FragCoord.xy / vec2(cellW, cellH));
    vec2 cellUV = fract(gl_FragCoord.xy / vec2(cellW, cellH));

    // Per-column speed and offset.
    float colHash = hash(vec2(cell.x, 0.0));
    float colSpeed = 1.0 + colHash * 3.0;
    float colOffset = colHash * 100.0;

    // Falling position.
    float fall = mod(cell.y + time * colSpeed + colOffset, 40.0);

    // Trail intensity — bright at head, fading behind.
    float intensity = smoothstep(20.0, 0.0, fall) * smoothstep(-1.0, 0.0, fall);

    // Flicker per character cell.
    float charHash = hash(cell + floor(time * 4.0));
    intensity *= 0.7 + 0.3 * charHash;

    // Bright head.
    float head = smoothstep(1.0, 0.0, fall) * 2.0;

    vec3 col = bgCol + rainCol * intensity;
    col += vec3(head * 0.5); // white-ish head glow

    fragColor = vec4(col, 1.0);
}
