# Shader Wallpapers

`oasis-shader` ships a library of Shadertoy-style fragment shaders used as
animated wallpapers. Each shader has two implementations:

- A **GLSL fragment shader** rendered on the GPU via `glow` (OpenGL on
  desktop, WebGL2 in the browser). Selected by the `gl-native` and `gl-web`
  Cargo features.
- A **Rust software renderer** that mirrors the GLSL math per-pixel on the
  CPU. It is always compiled (there is no feature flag for it) and is the
  fallback used by the PSP, SDL, WASM and UE5/FFI backends whenever there is
  no GL context. The optional `parallel` feature shades its rows on a rayon
  pool (desktop only).

The CPU and GPU paths produce visually equivalent output for each shader,
which is what lets the screenshot regression suite cover both backends with
the same fixture.

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
  (the skin-facing name), `glsl()` (fragment source), `from_name()` and
  `FromStr`. Both renderers dispatch on it.
- `SoftwareShaderRenderer` (`software.rs`) — CPU fallback. Renders at
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

The shaders use a Shadertoy-compatible uniform set. The Rust code names them
with the `u_` prefix (`gl_render` in `lib.rs`):

| Uniform | Purpose |
| --- | --- |
| `u_time` | Elapsed seconds (`f32`). |
| `u_resolution` | Viewport size (`vec2`). |
| `u_color1`, `u_color2`, `u_color3` | Theme colours from `ShaderParams.colors`. |
| `u_speed`, `u_contrast` | Per-shader floats from `ShaderParams.floats`. |
| `u_spin_speed`, `u_spin_amount`, `u_spin_ease` | Balatro-specific. |
| `u_pixel_filter` | Optional posterise filter. |
| `u_is_rotate` | Toggles auxiliary rotation. |
| `u_lighting`, `u_size` | Per-shader extras. |

`fragCoord` is sourced from `gl_FragCoord` on GPU and from loop indices on
CPU.

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
3. Upscale nearest-neighbour to the target dimensions (`upscale` / `upscale_nn` in `software.rs`).

The reduced internal size is the entire performance story: every shader
ships a software path because PSP and headless tests both need one, and 1/9
the pixel count is what makes that affordable. The PSP test budget comment
in the `software.rs` tests calls out 20–40 ms per frame on hardware against a
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

5. **Test.** Mirror the existing per-shader tests in `software.rs`
   asserting the buffer is fully written and that obvious params change
   visible output.

If you want a runtime-loadable extension instead of edit-the-registry, the
hook to extend would be `register()` to accept external GLSL plus a software
closure — there is no such API today.
