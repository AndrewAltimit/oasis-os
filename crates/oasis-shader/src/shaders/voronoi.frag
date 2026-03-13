#version 300 es
precision highp float;

// Animated Voronoi cells.
// Port of https://www.shadertoy.com/view/WdlyRS

uniform float u_time;
uniform vec2 u_resolution;
uniform vec4 u_color1;
uniform vec4 u_color2;
uniform float u_speed;
uniform float u_size;

out vec4 fragColor;

vec2 ran(vec2 uv) {
    uv *= vec2(dot(uv, vec2(127.1, 311.7)), dot(uv, vec2(227.1, 521.7)));
    return 1.0 - fract(tan(cos(uv) * 123.6) * 3533.3)
               * fract(tan(cos(uv) * 123.6) * 3533.3);
}

vec2 pt(float t, vec2 id) {
    return sin(t * (ran(id + 0.5) - 0.5) + ran(id - 20.1) * 8.0) * 0.5;
}

void main() {
    float speed = u_speed > 0.0 ? u_speed : 1.0;
    float size  = u_size  > 0.0 ? u_size  : 30.0;
    float t = u_time * speed * 2.0;

    vec3 col1 = u_color1.a > 0.0 ? u_color1.rgb
                                  : vec3(193.0, 41.0, 46.0) / 255.0;
    vec3 col2 = u_color2.a > 0.0 ? u_color2.rgb
                                  : vec3(241.0, 211.0, 2.0) / 255.0;

    vec2 uv = (gl_FragCoord.xy - 0.5 * u_resolution.xy) / u_resolution.x;
    vec2 off = u_time / vec2(50.0, 30.0);
    uv += off;
    uv *= size;

    vec2 gv = fract(uv) - 0.5;
    vec2 id = floor(uv);

    float mindist = 1e9;
    vec2 vorv;
    for (float i = -1.0; i <= 1.0; i++) {
        for (float j = -1.0; j <= 1.0; j++) {
            vec2 offv = vec2(i, j);
            float dist = length(gv + pt(t, id + offv) - offv);
            if (dist < mindist) {
                mindist = dist;
                vorv = (id + pt(t, id + offv) + offv) / size - off;
            }
        }
    }

    vec3 col = mix(col1, col2,
                   clamp(vorv.x * 2.2 + vorv.y, -1.0, 1.0) * 0.5 + 0.5);
    fragColor = vec4(col, 1.0);
}
