//! The built-in shader catalogue: one enum shared by the GPU and CPU paths.

use std::fmt;
use std::str::FromStr;

use crate::shaders;

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
