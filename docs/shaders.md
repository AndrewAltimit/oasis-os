# Shader Wallpapers

`oasis-shader` ships a library of Shadertoy-style fragment shaders used as
animated wallpapers. Each shader has two implementations:

- A **Rust software renderer** (`software/`) that evaluates the shader math
  per pixel on the CPU. It is always compiled (there is no feature flag for
  it) and it is **the implementation every shipping backend draws with**:
  the SDL, WASM, UE5/FFI and PSP backends all use
  `SoftwareShaderRenderer`. The optional `parallel` feature shades its rows
  on a rayon pool (desktop only).
- A **GLSL fragment shader** (`shaders/*.frag`), the GPU twin of each CPU
  kernel, rendered via `glow` (OpenGL on desktop, WebGL2 in the browser)
  behind the `gl-native` / `gl-web` features. No in-tree backend
  instantiates `ShaderRenderer` today; it is kept for GPU hosts.

The CPU port is canonical (skins are tuned against it and the golden
checksums pin it); the GLSL follows it. The two share:

- **Orientation**: top-left origin, y down. The CPU's row 0 is the first
  row of the returned buffer; every `.frag` builds
  `vec2 fragCoord = vec2(gl_FragCoord.x, u_resolution.y - gl_FragCoord.y);`
  and uses only that.
- **Parameters**: `BuiltinShader::default_colors()` / `float_params()` are
  the one table of inputs and defaults; the CPU kernels read them through
  `BuiltinShader::color(params, slot)` / `float(params, key)` and the GPU
  `gl_render` uploads the same resolved values.
- **Math**: the same hashes (`fract(sin(dot(p, k)) * 43758.5453)`, the CPU's
  cheaper Voronoi hash), constants and edge cases (a non-positive Voronoi /
  city-lights `size` falls back to 30 on both). The GLSL never calls
  `smoothstep` with `edge0 > edge1` (undefined in GLSL) and always writes
  an opaque, clamped colour.

Tests in `software/tests.rs` pin this: `glsl_uniforms_match_cpu_parameters`
(each `.frag` declares exactly `u_time`, `u_resolution`, one `u_colorN` per
color slot and one `u_<key>` per float), `glsl_sources_use_top_left_origin`,
and `every_declared_parameter_affects_cpu_output`. Bit-identical GPU output
is not a goal: GPU `sin` precision differs from the host libm.

## Public types

In `crates/oasis-shader/src/lib.rs`:

- `ShaderRenderer` — GPU renderer; owns the `glow::Context`, shader
  programs, FBO, and a reusable pixel buffer. Created with
  `ShaderRenderer::new(gl, w, h)` which auto-registers every built-in
  shader.
- `ShaderParams` — re-exported from `oasis-types::shader::ShaderParams`. A
  parameter bag with `colors: Vec<[f32; 4]>` and `floats: HashMap<String,
  f32>` consumed by both renderers.
- `BuiltinShader` (`registry.rs`) — the catalogue enum. `ALL`, `name()`
  (the skin-facing name), `glsl()` (fragment source), `from_name()`,
  `FromStr`, and the parameter table: `default_colors()`, `float_params()`,
  `color(params, slot)` and `float(params, key)` (resolve with defaults; a
  key the shader does not declare takes another built-in's default, else
  0). Both renderers dispatch on it.
- `SoftwareShaderRenderer` (`software/mod.rs`; kernels grouped in
  `swirl.rs` (balatro), `cells.rs` (voronoi, city lights), `waves.rs`
  (ocean, calm) and `procedural.rs` (starfield, plasma, matrix rain)) —
  the CPU renderer. Renders at
  `width/3 × height/3` (`RENDER_SCALE = 3`, coarser for very large outputs),
  then upscales nearest-neighbour to the target buffer.
  `render_builtin(BuiltinShader, ..)` dispatches on the enum;
  `render_shader(name, ..)` parses the name and falls back to Balatro (logged
  once) for unknown names, while the GPU `ShaderRenderer` returns an error
  for a name it has not registered.

## Built-in shaders

The authoritative list is `BuiltinShader::ALL` in `registry.rs`, backed by
the `include_str!` constants in `shaders/mod.rs`. There are eight:

| Skin name | Variant | Effect |
|---|---|---|
| `balatro` | `Balatro` | Balatro-style paint swirl (the default) |
| `voronoi` | `Voronoi` | Animated Voronoi cells |
| `city_lights` | `CityLights` | Night-time city lights |
| `ocean_waves` | `OceanWaves` | Layered ocean waves |
| `calm_waves` | `CalmWaves` | Slow, low-contrast waves |
| `starfield` | `Starfield` | Scrolling starfield |
| `plasma` | `Plasma` | Sine-wave plasma |
| `matrix_rain` | `MatrixRain` | Digital rain |

Each GPU shader has a paired `render_<name>` method on
`SoftwareShaderRenderer` for the CPU path.

## Shader inputs

Every shader gets `u_time` (elapsed seconds) and `u_resolution` (viewport
size). Color slot `i` is `ShaderParams.colors[i]` / uniform `u_color{i+1}`;
float `key` is `ShaderParams.floats["key"]` / uniform `u_<key>`. Per shader
(defaults from `registry.rs`):

| Shader | Color slots | Floats (default) |
| --- | --- | --- |
| `balatro` | 1 main, 2 accent, 3 dark | `speed` 1, `contrast` 3.5, `spin_speed` 1, `spin_amount` 0.25, `pixel_filter` 745, `is_rotate` 0, `lighting` 0.4, `spin_ease` 1 |
| `voronoi` | 1, 2 (blend ends) | `speed` 1, `size` 30 |
| `city_lights` | none | `speed` 1, `size` 30 |
| `ocean_waves` | 1 gradient top, 2 gradient bottom, 3 waves | `speed` 1 |
| `calm_waves` | 1 sky | `speed` 1 |
| `starfield` | 1 stars, 2 background | `speed` 1 |
| `plasma` | 1, 2, 3 (palette) | `speed` 1 |
| `matrix_rain` | 1 rain, 2 background | `speed` 1 |

`speed` is used as given (0 freezes, negative reverses). `fragCoord` is the
y-down pixel position: flipped `gl_FragCoord` on the GPU, the low-res cell
centre on the CPU.

## Execution model

### GPU path

1. `ShaderRenderer::render_to_screen(name, time, params)` — render directly
   to the bound default framebuffer.
2. `ShaderRenderer::render_to_pixels(name, time, params)` — render to an
   offscreen FBO, read back via `glReadPixels`, flip rows so the buffer is
   top-down (`gl_read_pixels`), and return a slice.

The render is always a fullscreen triangle from `quad.rs`. There is no
multi-pass support today.

### Software path

1. Compute size at `RENDER_SCALE = 3` reduction.
2. Loop pixel-by-pixel evaluating the per-shader math in Rust.
3. Upscale nearest-neighbour to the target dimensions (`upscale` /
   `upscale_nn` in `software/mod.rs`).

`golden_output_checksums` pins every kernel's output bit-for-bit. It runs
only on x86_64 Linux (the CI container), where the goldens were captured:
glibc and the MSVC CRT round some `sin` results differently, and the hash
shaders amplify that last bit.

The reduced internal size is the entire performance story: every shader
ships a software path because PSP and headless tests both need one, and 1/9
the pixel count is what makes that affordable. The PSP test budget comment
in the `software/tests.rs` tests calls out 20–40 ms per frame on hardware against a
33 ms / 30 fps target.

There is no caching of any kind. Every call re-evaluates the entire frame.
Pixel buffers are pre-allocated in the renderer and resized on viewport
change (`resize` on either renderer).

## Skin integration

Shaders are selected by name from skin TOML. `ShaderParams.colors` is filled
from the skin's theme palette and `ShaderParams.floats` from per-shader
overrides. The tests in `crates/oasis-backend-psp/src/skins.rs` show the canonical
parameter bundles for the shipped skins (Balatro, Terminal, etc.) — those
tables are also the place to look when authoring a new wallpaper preset.

## Adding a new shader

Built-in shaders are an enum, so the compiler points at every match that
needs a new arm. Adding a shader called `my_effect` touches:

1. **GLSL.** Create `crates/oasis-shader/src/shaders/my_effect.frag`. Use
   `#version 300 es` (the project targets GLSL ES 3.00 for WebGL2
   compatibility) and the existing uniform names. Crib `balatro.frag` for
   structure.

2. **`shaders/mod.rs`.** Add a `pub const MY_EFFECT_FRAG: &str =
   include_str!("my_effect.frag");`.

3. **`registry.rs`.** Add a `MyEffect` variant to `BuiltinShader`, append
   it to `ALL` (which `ShaderRenderer::new` compiles at startup), and add
   its arms to `name()` (`"my_effect"`) and `glsl()`
   (`shaders::MY_EFFECT_FRAG`).

4. **CPU port.** Add a `render_my_effect(time, params)` method on
   `SoftwareShaderRenderer` and its arm in `render_builtin` (the match is
   exhaustive, so this is required). Reuse the helpers (`hash2`, `fract`,
   `lerp`, `smoothstep`) already defined in that file.

5. **Test.** Mirror the existing per-shader tests in `software/tests.rs`
   asserting the buffer is fully written and that obvious params change
   visible output.

If you want a runtime-loadable extension instead of edit-the-registry, the
hook to extend would be `register()` to accept external GLSL plus a software
closure — there is no such API today.
