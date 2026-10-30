//! `impl_sdi_defaults!`: empty extension-trait impls for a backend type.

/// Implement every [`SdiBackend`](crate::backend::SdiBackend) extension
/// trait for a type with an empty `impl` block, so it picks up all the
/// default methods.
///
/// A type that implements [`SdiCore`](crate::backend::SdiCore) and invokes
/// this macro satisfies `SdiBackend` through the blanket impl. This is the
/// usual way to write a test mock, a null backend or a backend that only
/// accelerates a few traits:
///
/// ```
/// use oasis_types::backend::{Color, SdiBackend, SdiCore, SdiShapes, TextureId};
/// use oasis_types::error::Result;
///
/// struct Null;
///
/// impl SdiCore for Null {
///     fn init(&mut self, _: u32, _: u32) -> Result<()> { Ok(()) }
///     fn clear(&mut self, _: Color) -> Result<()> { Ok(()) }
///     fn blit(&mut self, _: TextureId, _: i32, _: i32, _: u32, _: u32) -> Result<()> { Ok(()) }
///     fn fill_rect(&mut self, _: i32, _: i32, _: u32, _: u32, _: Color) -> Result<()> { Ok(()) }
///     fn draw_text(&mut self, _: &str, _: i32, _: i32, _: u16, _: Color) -> Result<()> {
///         Ok(())
///     }
///     fn swap_buffers(&mut self) -> Result<()> { Ok(()) }
///     fn load_texture(&mut self, _: u32, _: u32, _: &[u8]) -> Result<TextureId> {
///         Ok(TextureId(0))
///     }
///     fn destroy_texture(&mut self, _: TextureId) -> Result<()> { Ok(()) }
///     fn set_clip_rect(&mut self, _: i32, _: i32, _: u32, _: u32) -> Result<()> { Ok(()) }
///     fn reset_clip_rect(&mut self) -> Result<()> { Ok(()) }
///     fn measure_text(&self, text: &str, _: u16) -> u32 { text.len() as u32 * 8 }
///     fn read_pixels(&self, _: i32, _: i32, _: u32, _: u32) -> Result<Vec<u8>> {
///         Ok(Vec::new())
///     }
///     fn shutdown(&mut self) -> Result<()> { Ok(()) }
/// }
///
/// // All nine extension traits with their defaults, except `SdiShapes`,
/// // which this type writes itself.
/// oasis_types::impl_sdi_defaults!(Null; except SdiShapes);
/// impl SdiShapes for Null {}
///
/// fn takes_backend(_: &mut dyn SdiBackend) {}
/// takes_backend(&mut Null);
/// ```
///
/// `impl_sdi_defaults!(T)` implements all nine traits;
/// `impl_sdi_defaults!(T; except A, B)` skips the listed ones (by their
/// bare trait names) so the type can provide its own `impl` for them.
#[macro_export]
macro_rules! impl_sdi_defaults {
    // A trait named in the skip list: emit nothing.
    (@one $ty:ty; SdiShapes; [SdiShapes $($rest:ident)*]) => {};
    (@one $ty:ty; SdiGradients; [SdiGradients $($rest:ident)*]) => {};
    (@one $ty:ty; SdiAlpha; [SdiAlpha $($rest:ident)*]) => {};
    (@one $ty:ty; SdiText; [SdiText $($rest:ident)*]) => {};
    (@one $ty:ty; SdiTextures; [SdiTextures $($rest:ident)*]) => {};
    (@one $ty:ty; SdiClipTransform; [SdiClipTransform $($rest:ident)*]) => {};
    (@one $ty:ty; SdiVector; [SdiVector $($rest:ident)*]) => {};
    (@one $ty:ty; SdiBatch; [SdiBatch $($rest:ident)*]) => {};
    (@one $ty:ty; SdiRenderTarget; [SdiRenderTarget $($rest:ident)*]) => {};
    // Head of the skip list is some other trait: keep looking.
    (@one $ty:ty; $tr:ident; [$head:ident $($rest:ident)*]) => {
        $crate::impl_sdi_defaults!(@one $ty; $tr; [$($rest)*]);
    };
    // Not skipped.
    (@one $ty:ty; $tr:ident; []) => {
        impl $crate::backend::$tr for $ty {}
    };
    ($ty:ty; except $($skip:ident),* $(,)?) => {
        $crate::impl_sdi_defaults!(@one $ty; SdiShapes; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiGradients; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiAlpha; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiText; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiTextures; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiClipTransform; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiVector; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiBatch; [$($skip)*]);
        $crate::impl_sdi_defaults!(@one $ty; SdiRenderTarget; [$($skip)*]);
    };
    ($ty:ty) => {
        $crate::impl_sdi_defaults!($ty; except);
    };
}
