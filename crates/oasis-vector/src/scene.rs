//! Vector scene graph.
//!
//! A `VectorScene` is a collection of `VectorOp`s with a defined viewport size.
//! Scenes can be composed, translated, and scaled to fit different screen
//! resolutions.

use oasis_types::backend::Color;

use crate::op::VectorOp;

/// A self-contained vector scene with a defined viewport.
///
/// The `width` and `height` define the design-time coordinate space.
/// When rendered at a different size, the rasterizer can scale coordinates
/// proportionally.
#[derive(Debug, Clone)]
pub struct VectorScene {
    /// Design-time viewport width.
    pub width: u32,
    /// Design-time viewport height.
    pub height: u32,
    /// Drawing operations in paint order (back to front).
    pub ops: Vec<VectorOp>,
}

impl VectorScene {
    /// Create an empty scene with the given viewport dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ops: Vec::new(),
        }
    }

    /// Add an operation to the scene.
    pub fn push(&mut self, op: VectorOp) {
        self.ops.push(op);
    }

    /// Add all operations from another scene, translated to the given offset.
    pub fn embed(&mut self, x: i32, y: i32, other: &VectorScene) {
        if other.ops.is_empty() {
            return;
        }
        self.ops.push(VectorOp::translated(x, y, other.ops.clone()));
    }

    /// Recolor all operations in the scene.
    pub fn recolor(&mut self, color: Color) {
        for op in &mut self.ops {
            op.recolor(color);
        }
    }

    /// Apply alpha modulation to all operations.
    pub fn modulate_alpha(&mut self, alpha: u8) {
        for op in &mut self.ops {
            op.modulate_alpha(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_scene() {
        let scene = VectorScene::new(480, 272);
        assert_eq!(scene.width, 480);
        assert_eq!(scene.height, 272);
        assert!(scene.ops.is_empty());
    }

    #[test]
    fn test_push() {
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::FillRect {
            x: 0,
            y: 0,
            w: 50,
            h: 50,
            color: Color::WHITE,
        });
        assert_eq!(scene.ops.len(), 1);
    }

    #[test]
    fn test_embed() {
        let mut parent = VectorScene::new(200, 200);
        let mut child = VectorScene::new(50, 50);
        child.push(VectorOp::FillCircle {
            cx: 25,
            cy: 25,
            radius: 10,
            color: Color::WHITE,
        });
        parent.embed(100, 50, &child);
        assert_eq!(parent.ops.len(), 1);
        let __out = &parent.ops[0];
        let VectorOp::Group { translate, ops, .. } = __out else {
            panic!("expected Group, got {__out:?}");
        };
        assert_eq!(*translate, (100, 50));
        assert_eq!(ops.len(), 1);
    }

    #[test]
    fn test_embed_empty() {
        let mut parent = VectorScene::new(200, 200);
        let child = VectorScene::new(50, 50);
        parent.embed(0, 0, &child);
        assert!(parent.ops.is_empty());
    }
}
