//! `SdiRenderTarget` for the SDL3 backend: offscreen `TEXTUREACCESS_TARGET`
//! textures, the bind chain, and compositing them back with CSS blend
//! modes.

use sdl3::render::Texture;

use oasis_types::backend::stacks::DrawState;
use oasis_types::backend::{
    BackendErrExt, BlendMode, RenderTargetId, SdiRenderTarget, render_target_not_found, unsupported,
};
use oasis_types::error::{OasisError, Result};

use super::{SdlBackend, frect};

/// The error for a render-target call on a target that is currently bound
/// (on the bind chain).
fn bound_target_error(op: &str, id: RenderTargetId) -> OasisError {
    OasisError::Backend(format!("{op}: render target {} is bound", id.0).into())
}

/// The surface a render-target bind replaced, restored on unbind.
///
/// SDL3 keeps the clip rectangle per render target, so the parent's
/// applied clip survives the bind on the SDL side; the Rust-side stacks
/// and the viewport size are saved here.
pub(crate) struct BoundFrom {
    /// Render target bound before (`None` = the window).
    pub(crate) prev_id: Option<u64>,
    /// The parent surface's clip and translate stacks.
    pub(crate) state: DrawState,
    /// The parent surface's size (`viewport_size`).
    pub(crate) viewport: (u32, u32),
    /// The window was resized while this bind was active: its SDL-side
    /// clip is stale and is lifted when the unbind returns to it.
    pub(crate) drop_parent_clip: bool,
}

impl SdlBackend {
    /// Whether `id` is the bound render target or one of the targets the
    /// current bind chain will return to.
    fn is_bound(&self, id: RenderTargetId) -> bool {
        self.current_render_target == Some(id.0)
            || self
                .render_target_bind_stack
                .iter()
                .any(|b| b.prev_id == Some(id.0))
    }
}

// -------------------------------------------------------------------
// SdiRenderTarget: Offscreen compositing layers (compositor PR4)
// -------------------------------------------------------------------

/// Map a CSS blend mode onto the SDL3 blend mode that composites a
/// render target with it.
///
/// Layer contents are straight-alpha primitives blended (`BLEND`) into a
/// target cleared to transparent black, which leaves the target holding
/// *premultiplied* colors (`rgb * a`). They must therefore be composited
/// with premultiplied operators:
///
/// - `Normal` -> `BLEND_PREMULTIPLIED` (`dst = src + dst * (1 - srcA)`).
///   Plain `BLEND` multiplied translucent layer content by its alpha a
///   second time, so e.g. a 50% red fill inside an opacity layer came out
///   at 25% (visibly darker than the same fill painted directly).
/// - `Multiply` -> `MUL` (`dst = dst * src + dst * (1 - srcA)`), which is
///   exactly CSS `multiply` for a premultiplied source, including the
///   layer's transparent pixels, which leave the backdrop untouched.
///
/// SDL has no other CSS modes; they degrade to `Normal` (the software
/// rasterizer does the same, so all backends agree).
fn sdl_blend_for(mode: BlendMode) -> sdl3::sys::blendmode::SDL_BlendMode {
    use sdl3::sys::blendmode as bm;
    match mode {
        BlendMode::Multiply => bm::SDL_BLENDMODE_MUL,
        _ => bm::SDL_BLENDMODE_BLEND_PREMULTIPLIED,
    }
}

impl SdiRenderTarget for SdlBackend {
    fn create_render_target(&mut self, w: u32, h: u32) -> Result<RenderTargetId> {
        if w == 0 || h == 0 {
            return Err(OasisError::Backend(
                format!("create_render_target: zero dimension ({w}x{h})").into(),
            ));
        }
        // Create an ABGR8888 texture with TARGET access so the
        // renderer can draw into it.
        let raw_renderer = self.canvas.raw();
        // SAFETY: raw_renderer is a valid SDL_Renderer pointer owned
        // by `canvas`. SDL_CreateTexture does not retain the pointer;
        // it returns a fresh SDL_Texture on success or null on failure.
        let raw_tex = unsafe {
            sdl3::sys::render::SDL_CreateTexture(
                raw_renderer,
                sdl3::sys::pixels::SDL_PIXELFORMAT_ABGR8888,
                sdl3::sys::render::SDL_TEXTUREACCESS_TARGET,
                w as i32,
                h as i32,
            )
        };
        if raw_tex.is_null() {
            return Err(OasisError::Backend(
                format!("SDL_CreateTexture (target {w}x{h}) failed").into(),
            ));
        }
        // SDL leaves a new target texture's contents undefined, and the
        // Direct3D 11 driver hands back recycled VRAM: a layer that draws
        // nothing (or only part of its bounds) would composite whatever a
        // previously destroyed same-size target held, e.g. last frame's
        // `mask-image` layer showing up under a `filter: blur` element.
        // Callers expect a fresh target to be fully transparent, so clear
        // it once here, then restore whichever target was bound.
        let restore: *mut sdl3::sys::render::SDL_Texture = self
            .current_render_target
            .and_then(|id| self.render_targets.get(&id))
            .map_or(std::ptr::null_mut(), |tex| tex.raw());
        // SAFETY: raw_renderer is the canvas's valid renderer; raw_tex was
        // just created on it with TARGET access, and `restore` is either
        // null (the window) or a live target texture owned by this backend.
        // SDL_RenderClear ignores the blend mode and writes the draw color
        // (transparent black) straight into the target.
        let cleared = unsafe {
            use sdl3::sys::render::{SDL_RenderClear, SDL_SetRenderDrawColor, SDL_SetRenderTarget};
            let ok = SDL_SetRenderTarget(raw_renderer, raw_tex)
                && SDL_SetRenderDrawColor(raw_renderer, 0, 0, 0, 0)
                && SDL_RenderClear(raw_renderer);
            SDL_SetRenderTarget(raw_renderer, restore) && ok
        };
        // The raw draw-color change bypassed the cached color.
        self.last_draw_color = None;
        if !cleared {
            // SAFETY: raw_tex is the valid texture created above and is
            // not referenced anywhere else yet.
            unsafe { sdl3::sys::render::SDL_DestroyTexture(raw_tex) };
            return Err(OasisError::Backend(
                format!("clearing new render target ({w}x{h}) failed").into(),
            ));
        }
        // SAFETY: raw_tex is a valid SDL_Texture; wrap in Rust handle.
        let tex: Texture<'_> = unsafe { self.texture_creator.raw_create_texture(raw_tex) };
        // SAFETY: Erase the lifetime to `'static` using the same pattern
        // as regular textures on this backend. Correctness relies on
        // `render_targets.clear()` in `Drop` ordering before the
        // texture_creator is dropped.
        let tex: Texture<'static> = unsafe { std::mem::transmute(tex) };
        let id = self.next_render_target_id;
        self.next_render_target_id += 1;
        self.render_targets.insert(id, tex);
        Ok(RenderTargetId(id))
    }

    fn bind_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        // A bound target is on the bind chain already: re-binding it would
        // clear its SDL-side clip, which the unbind cannot restore. UE5
        // and WASM take a bound target out of storage, so they reject it
        // the same way.
        if self.is_bound(id) {
            return Err(bound_target_error("bind_render_target", id));
        }
        let (raw_tex, size) = match self.render_targets.get(&id.0) {
            Some(tex) => (tex.raw(), (tex.width(), tex.height())),
            None => return Err(render_target_not_found(id)),
        };
        // SAFETY: raw_tex is a valid SDL_Texture owned by this backend.
        // SDL_SetRenderTarget accepts any texture created with
        // `TEXTUREACCESS_TARGET` from the same renderer.
        let ok = unsafe { sdl3::sys::render::SDL_SetRenderTarget(self.canvas.raw(), raw_tex) };
        if !ok {
            return Err(OasisError::Backend(
                "SDL_SetRenderTarget (bind) failed".into(),
            ));
        }
        // SDL3 keeps the clip per target, so the parent's applied clip
        // waits on the parent's view; a clip left on this target by an
        // earlier bind must not survive (a bound target starts unclipped
        // and untranslated, see `SdiRenderTarget`).
        self.canvas.set_clip_rect(None);
        self.render_target_bind_stack.push(BoundFrom {
            prev_id: self.current_render_target,
            state: DrawState::take(&mut self.clip_stack, &mut self.translate_stack),
            viewport: (self.viewport_w, self.viewport_h),
            drop_parent_clip: false,
        });
        (self.viewport_w, self.viewport_h) = size;
        self.current_render_target = Some(id.0);
        Ok(())
    }

    fn unbind_render_target(&mut self) -> Result<()> {
        let bound = self.render_target_bind_stack.pop().ok_or_else(|| {
            OasisError::Backend("unbind_render_target: bind stack underflow".into())
        })?;
        let raw_tex = bound
            .prev_id
            .and_then(|id| self.render_targets.get(&id))
            .map_or(std::ptr::null_mut(), |tex| tex.raw());
        // SAFETY: raw_tex is either null (framebuffer) or a valid
        // texture owned by this backend that has outlived the current
        // bind scope.
        let ok = unsafe { sdl3::sys::render::SDL_SetRenderTarget(self.canvas.raw(), raw_tex) };
        // The Rust-side state is restored even if SDL failed, so the
        // stacks never stay desynchronized from the bind stack.
        bound
            .state
            .restore(&mut self.clip_stack, &mut self.translate_stack);
        (self.viewport_w, self.viewport_h) = bound.viewport;
        self.current_render_target = bound.prev_id;
        if bound.drop_parent_clip {
            self.apply_clip(None);
        }
        if !ok {
            return Err(OasisError::Backend(
                "SDL_SetRenderTarget (unbind) failed".into(),
            ));
        }
        Ok(())
    }

    fn composite_render_target(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
        blend: BlendMode,
        opacity: f32,
    ) -> Result<()> {
        debug_assert!(
            (0.0..=1.0).contains(&opacity),
            "opacity must be in [0.0, 1.0], got {opacity}"
        );
        if self.is_bound(id) {
            return Err(bound_target_error("composite_render_target", id));
        }
        let Self {
            canvas,
            render_targets,
            ..
        } = self;
        let tex = render_targets
            .get_mut(&id.0)
            .ok_or_else(|| render_target_not_found(id))?;
        // SAFETY: the raw pointer comes from a live texture owned by
        // `render_targets` (borrowed above) and is only used for this call.
        let premultiplied =
            unsafe { sdl3::sys::render::SDL_SetTextureBlendMode(tex.raw(), sdl_blend_for(blend)) };
        let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        if premultiplied {
            // Opacity scales a premultiplied source uniformly: color and
            // alpha alike.
            tex.set_color_mod(alpha, alpha, alpha);
        } else {
            // Renderer without premultiplied / MUL support: straight alpha
            // (translucent layer content comes out too dark).
            tex.set_blend_mode(sdl3::render::BlendMode::Blend);
            tex.set_color_mod(255, 255, 255);
        }
        tex.set_alpha_mod(alpha);
        let r = canvas
            .copy(tex, None, Some(frect(dst_x, dst_y, dst_w, dst_h)))
            .backend_err();
        tex.set_alpha_mod(255);
        tex.set_color_mod(255, 255, 255);
        tex.set_blend_mode(sdl3::render::BlendMode::Blend);
        r
    }

    fn composite_render_target_premultiplied(
        &mut self,
        id: RenderTargetId,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<()> {
        if self.is_bound(id) {
            return Err(bound_target_error(
                "composite_render_target_premultiplied",
                id,
            ));
        }
        let Self {
            canvas,
            render_targets,
            ..
        } = self;
        let tex = render_targets
            .get_mut(&id.0)
            .ok_or_else(|| render_target_not_found(id))?;
        // The safe sdl3 `BlendMode` enum does not expose
        // SDL_BLENDMODE_BLEND_PREMULTIPLIED, so set it through the sys
        // API. The call doubles as the capability probe: SDL rejects
        // blend modes the active renderer cannot execute, and the
        // resulting `Err` makes callers (the vector-layer bake cache)
        // fall back to immediate-mode drawing instead of compositing
        // with wrong alpha.
        let raw_tex = tex.raw();
        // SAFETY: raw_tex is a valid SDL_Texture owned by this backend
        // (fetched from `render_targets` above and kept alive by it).
        let ok = unsafe {
            sdl3::sys::render::SDL_SetTextureBlendMode(
                raw_tex,
                sdl3::sys::blendmode::SDL_BLENDMODE_BLEND_PREMULTIPLIED,
            )
        };
        if !ok {
            return Err(unsupported("premultiplied blending on this SDL renderer"));
        }
        tex.set_alpha_mod(255);
        tex.set_color_mod(255, 255, 255);
        let r = canvas
            .copy(tex, None, Some(frect(dst_x, dst_y, dst_w, dst_h)))
            .backend_err();
        // Restore the straight-alpha default the plain composite path
        // (and any other texture user) expects.
        tex.set_blend_mode(sdl3::render::BlendMode::Blend);
        r
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) -> Result<()> {
        // Destroying a still-bound target would leave the bind chain
        // pointing at a freed texture.
        if self.is_bound(id) {
            return Err(bound_target_error("destroy_render_target", id));
        }
        if self.render_targets.remove(&id.0).is_none() {
            return Err(render_target_not_found(id));
        }
        Ok(())
    }

    fn supports_render_targets(&self) -> bool {
        true
    }

    /// Deliberately `false`: `read_render_target` is not implemented.
    ///
    /// A GPU readback (`SDL_RenderReadPixels` with the target bound)
    /// stalls the pipeline once per layer per frame. The browser only
    /// reads layers back for `backdrop-filter` / `filter` / `mask` and,
    /// on `false`, uses its static-tint fallback, which keeps desktop
    /// frame times flat and the screenshot baselines unchanged.
    fn supports_render_target_readback(&self) -> bool {
        false
    }
}
