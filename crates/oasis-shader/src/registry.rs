//! The built-in shader catalogue: one enum shared by the GPU and CPU paths.

use std::fmt;
use std::str::FromStr;

use crate::shaders;
use crate::uniforms::ShaderParams;

/// A built-in wallpaper shader.
///
/// Both renderers dispatch on this enum: the GPU `ShaderRenderer` compiles
/// [`glsl`](Self::glsl) for every entry of [`ALL`](Self::ALL), and
/// [`SoftwareShaderRenderer`](crate::software::SoftwareShaderRenderer) has a
/// Rust port of each. Skins refer to shaders by [`name`](Self::name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinShader {
    /// Balatro-style swirling paint (the default).
    Balatro,
    /// Animated Voronoi cells.
    Voronoi,
    /// Night-time city lights.
    CityLights,
    /// Layered ocean waves.
    OceanWaves,
    /// Slow, low-contrast waves.
    CalmWaves,
    /// Scrolling starfield.
    Starfield,
    /// Classic plasma.
    Plasma,
    /// Falling "digital rain" glyph columns.
    MatrixRain,
}

impl BuiltinShader {
    /// Every built-in shader, in registration order.
    pub const ALL: [Self; 8] = [
        Self::Balatro,
        Self::Voronoi,
        Self::CityLights,
        Self::OceanWaves,
        Self::CalmWaves,
        Self::Starfield,
        Self::Plasma,
        Self::MatrixRain,
    ];

    /// The name skins use (`[effects] shader = "..."`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Balatro => "balatro",
            Self::Voronoi => "voronoi",
            Self::CityLights => "city_lights",
            Self::OceanWaves => "ocean_waves",
            Self::CalmWaves => "calm_waves",
            Self::Starfield => "starfield",
            Self::Plasma => "plasma",
            Self::MatrixRain => "matrix_rain",
        }
    }

    /// GLSL fragment shader source.
    pub fn glsl(self) -> &'static str {
        match self {
            Self::Balatro => shaders::BALATRO_FRAG,
            Self::Voronoi => shaders::VORONOI_FRAG,
            Self::CityLights => shaders::CITY_LIGHTS_FRAG,
            Self::OceanWaves => shaders::OCEAN_WAVES_FRAG,
            Self::CalmWaves => shaders::CALM_WAVES_FRAG,
            Self::Starfield => shaders::STARFIELD_FRAG,
            Self::Plasma => shaders::PLASMA_FRAG,
            Self::MatrixRain => shaders::MATRIX_RAIN_FRAG,
        }
    }

    /// Look up a shader by its skin name; `None` for unknown names.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Default RGBA (0..1) for each color slot the shader reads, in slot
    /// order: slot `i` is `params.colors[i]` and the GLSL uniform
    /// `u_color{i+1}`. The single source of the defaults for both
    /// renderers.
    pub fn default_colors(self) -> &'static [[f32; 4]] {
        match self {
            Self::Balatro => &[
                [0.871, 0.267, 0.251, 1.0], // #DE4440
                [0.0, 0.420, 0.706, 1.0],   // #006BB4
                [0.086, 0.137, 0.145, 1.0], // #162325
            ],
            Self::Voronoi => &[
                [193.0 / 255.0, 41.0 / 255.0, 46.0 / 255.0, 1.0],
                [241.0 / 255.0, 211.0 / 255.0, 2.0 / 255.0, 1.0],
            ],
            Self::CityLights => &[],
            // Gradient start (top row), gradient end (bottom row), waves.
            Self::OceanWaves => &[
                [0.0, 0.05, 0.4, 1.0],
                [0.0, 0.9, 0.9, 1.0],
                [0.6, 0.6, 0.6, 1.0],
            ],
            Self::CalmWaves => &[[102.0 / 255.0, 204.0 / 255.0, 1.0, 1.0]],
            Self::Starfield => &[[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.05, 1.0]],
            Self::Plasma => &[
                [1.0, 0.0, 0.5, 1.0],
                [0.0, 0.8, 1.0, 1.0],
                [1.0, 0.9, 0.0, 1.0],
            ],
            Self::MatrixRain => &[[0.0, 1.0, 0.3, 1.0], [0.0, 0.02, 0.0, 1.0]],
        }
    }

    /// The named float parameters the shader reads (`params.floats` key,
    /// GLSL uniform `u_<key>`) with their defaults.
    pub fn float_params(self) -> &'static [(&'static str, f32)] {
        match self {
            Self::Balatro => &[
                ("speed", 1.0),
                ("contrast", 3.5),
                ("spin_speed", 1.0),
                ("spin_amount", 0.25),
                ("pixel_filter", 745.0),
                ("is_rotate", 0.0),
                ("lighting", 0.4),
                ("spin_ease", 1.0),
            ],
            Self::Voronoi | Self::CityLights => &[("speed", 1.0), ("size", 30.0)],
            Self::OceanWaves
            | Self::CalmWaves
            | Self::Starfield
            | Self::Plasma
            | Self::MatrixRain => &[("speed", 1.0)],
        }
    }

    /// Resolve color slot `slot` from `params`, falling back to
    /// [`default_colors`](Self::default_colors). Slots past the defaults
    /// resolve to opaque black.
    pub fn color(self, params: &ShaderParams, slot: usize) -> [f32; 4] {
        params
            .colors
            .get(slot)
            .or_else(|| self.default_colors().get(slot))
            .copied()
            .unwrap_or([0.0, 0.0, 0.0, 1.0])
    }

    /// Resolve float parameter `key` from `params`, falling back to its
    /// [`float_params`](Self::float_params) default. A key this shader does
    /// not declare takes the default another built-in declares for it
    /// (custom GLSL programs share the uniform names), else 0.0.
    pub fn float(self, params: &ShaderParams, key: &str) -> f32 {
        params.floats.get(key).copied().unwrap_or_else(|| {
            std::iter::once(self)
                .chain(Self::ALL)
                .flat_map(|s| s.float_params())
                .find(|(k, _)| *k == key)
                .map_or(0.0, |&(_, v)| v)
        })
    }
}

impl fmt::Display for BuiltinShader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error returned when parsing an unknown shader name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownShader(pub String);

impl fmt::Display for UnknownShader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown built-in shader '{}'", self.0)
    }
}

impl std::error::Error for UnknownShader {}

impl FromStr for BuiltinShader {
    type Err = UnknownShader;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s).ok_or_else(|| UnknownShader(s.to_string()))
    }
}

/// Look up a built-in fragment shader source by name.
pub fn get_shader_source(name: &str) -> Option<&'static str> {
    BuiltinShader::from_name(name).map(BuiltinShader::glsl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for s in BuiltinShader::ALL {
            assert_eq!(BuiltinShader::from_name(s.name()), Some(s));
            assert_eq!(s.name().parse::<BuiltinShader>(), Ok(s));
            assert_eq!(s.to_string(), s.name());
            assert!(!s.glsl().is_empty());
        }
    }

    #[test]
    fn unknown_name() {
        assert_eq!(BuiltinShader::from_name("balatr0"), None);
        let err = "nope".parse::<BuiltinShader>().unwrap_err();
        assert_eq!(err.to_string(), "unknown built-in shader 'nope'");
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = BuiltinShader::ALL.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BuiltinShader::ALL.len());
    }
}
