#version 300 es
precision highp float;

// Standard uniforms.
uniform float u_time;
uniform vec2 u_resolution;

// Configurable parameters.
uniform vec4 u_color1;
uniform vec4 u_color2;
uniform vec4 u_color3;
uniform float u_speed;
uniform float u_contrast;
uniform float u_spin_speed;
uniform float u_spin_amount;
uniform float u_pixel_filter;
uniform float u_is_rotate;
uniform float u_lighting;
uniform float u_spin_ease;

out vec4 fragColor;

#define PI 3.14159265359

// Attempt to replicate https://www.shadertoy.com/view/XXtBRr
// Original by localthunk (https://www.playbalatro.com)
void main() {
    // Pixel quantisation for retro look.
    float pixel_size = length(u_resolution.xy) / u_pixel_filter;
    vec2 uv = (floor(gl_FragCoord.xy / pixel_size) * pixel_size
               - 0.5 * u_resolution.xy) / length(u_resolution.xy);
    float uv_len = length(uv);

    // Rotation: slow continuous spin when enabled.
    float rot = 1.0;
    if (u_is_rotate > 0.5) {
        rot = u_time * u_spin_speed * (2.0 * PI / 60.0);
    }

    // Polar-coordinate twist: angle shifts with distance from centre.
    float angle = atan(uv.y, uv.x) + rot
                  - u_spin_ease * 20.0
                    * (u_spin_amount * uv_len + (1.0 - u_spin_amount));
    vec2 mid = (u_resolution.xy / length(u_resolution.xy)) * 0.5;
    uv = vec2(uv_len * cos(angle) + mid.x,
              uv_len * sin(angle) + mid.y) - mid;

    // Scale up for distortion detail.
    uv *= 30.0;

    // Time-varying distortion phase.
    float phase = u_time * u_speed * (2.0 * PI / 10.0);

    vec2 uv2 = vec2(uv.x + uv.y);

    // 5-iteration distortion loop.
    for (int i = 0; i < 5; i++) {
        uv2 += sin(max(uv.x, uv.y)) + uv;
        uv  += 0.5 * vec2(cos(5.1123314 + 0.353 * uv2.y + phase),
                           sin(uv2.x - phase));
        uv  -= cos(uv.x + uv.y) - sin(uv.x * 0.711 - uv.y);
    }

    // Distance-based colour weighting.
    float contrast_mod = 0.25 * u_contrast + 0.5 * u_spin_amount + 1.2;
    float paint_res = clamp(length(uv) * 0.035 * contrast_mod, 0.0, 2.0);
    float c1p = max(0.0, 1.0 - contrast_mod * abs(1.0 - paint_res));
    float c2p = max(0.0, 1.0 - contrast_mod * abs(paint_res));
    float c3p = 1.0 - min(1.0, c1p + c2p);

    // Lighting highlights at colour boundaries.
    float light = (u_lighting - 0.2) * max(c1p * 5.0 - 4.0, 0.0)
                + u_lighting * max(c2p * 5.0 - 4.0, 0.0);

    // Final colour: base tint + weighted blend + highlights.
    float base_w = 0.3 / u_contrast;
    vec4 col = base_w * u_color1
             + (1.0 - base_w)
               * (u_color1 * c1p + u_color2 * c2p
                  + vec4(c3p * u_color3.rgb, c3p * u_color1.a))
             + light;

    fragColor = col;
}
