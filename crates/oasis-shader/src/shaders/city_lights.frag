#version 300 es
precision highp float;

// Animated colour grid with cell shadows.
// Port of https://www.shadertoy.com/view/wscGWl. GLSL twin of
// `software/cells.rs` (the CPU port every backend ships); keep the two
// in step.

uniform float u_time;
uniform vec2 u_resolution;
uniform float u_speed;
uniform float u_size;

out vec4 fragColor;

float rand(vec2 co) {
    return fract(sin(dot(co.xy, vec2(12.9898, 78.233))) * 43758.5453);
}

float getCellBright(float time, vec2 id) {
    return sin((time + 2.0) * rand(id) * 2.0) * 0.5 + 0.5;
}

void main() {
    // Top-left origin, y down (matches the CPU renderer).
    vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);

    // A non-positive size would divide by zero: use the default.
    float size = u_size > 0.0 ? u_size : 30.0;

    float mx = max(u_resolution.x, u_resolution.y);
    vec2 uv = fragCoord / mx;

    float time = u_time * 0.5 * u_speed;

    uv *= size;

    vec2 id = floor(uv);
    vec2 gv = fract(uv) - 0.5;

    float bright = getCellBright(u_time * u_speed, id);

    vec3 colorShift = vec3(rand(id) * 0.1);
    vec3 color = 0.6 + 0.5 * cos(time + (id.xyx * 0.1)
                                  + vec3(4.0, 2.0, 1.0) + colorShift);

    float shadow = 0.0;
    shadow += smoothstep(0.0, 0.7,
        gv.x * min(0.0, getCellBright(u_time * u_speed, vec2(id.x - 1.0, id.y))
                       - bright));
    shadow += smoothstep(0.0, 0.7,
       -gv.y * min(0.0, getCellBright(u_time * u_speed, vec2(id.x, id.y + 1.0))
                       - bright));
    color -= shadow * 0.4;

    color *= 1.0 - (bright * 0.2);

    fragColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
