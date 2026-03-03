//! SDI scene graph objects.
//!
//! An `SdiObject` is a named, positionable, blittable element in the scene
//! graph. SDI is deliberately flat -- no hierarchy, no parent-child, no
//! grouping. The window manager (when present) simulates hierarchy via
//! naming conventions.

use oasis_types::backend::{Color, TextureId};

/// Semantic role for accessibility screen readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityRole {
    /// A clickable button.
    Button,
    /// A heading / title.
    Heading,
    /// An item in a list.
    ListItem,
    /// A text input field.
    TextField,
    /// A menu item.
    MenuItem,
    /// An image or icon.
    Image,
    /// A grouping container.
    Group,
    /// A status indicator (e.g. battery, network).
    Status,
    /// A navigation container.
    Navigation,
}

/// A single object in the SDI scene graph.
#[derive(Debug, Clone)]
pub struct SdiObject {
    /// Unique name (used as the registry key).
    pub name: String,
    /// X position in virtual screen coordinates.
    pub x: i32,
    /// Y position in virtual screen coordinates.
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// Alpha (0 = fully transparent, 255 = fully opaque).
    pub alpha: u8,
    /// Z-order index (higher = drawn later = on top).
    pub z: i32,
    /// Whether this object is drawn.
    pub visible: bool,
    /// Optional texture handle. If `None`, the object draws as a solid `color`.
    pub texture: Option<TextureId>,
    /// Solid fill color (used when `texture` is `None` and `text` is `None`).
    pub color: Color,
    /// Optional text content. When set, the object renders text instead of a
    /// filled rectangle. The `color` field is used as the text color.
    pub text: Option<String>,
    /// Font size in pixels (used when `text` is `Some`).
    pub font_size: u16,
    /// Text color (separate from background fill color).
    pub text_color: Color,
    /// When true, this object is drawn in the overlay pass (on top of all
    /// base-layer objects). Matches PSIX's two-layer rendering model.
    pub overlay: bool,

    // -- Extended visual properties (all optional, None = legacy behavior) --
    /// Corner radius for rounded rectangles (pixels).
    pub border_radius: Option<u16>,
    /// Vertical gradient top color (used with `gradient_bottom`).
    pub gradient_top: Option<Color>,
    /// Vertical gradient bottom color (used with `gradient_top`).
    pub gradient_bottom: Option<Color>,
    /// Shadow elevation level (0 = none, 1-3 = increasingly prominent).
    pub shadow_level: Option<u8>,
    /// Stroke/outline width in pixels.
    pub stroke_width: Option<u16>,
    /// Stroke/outline color.
    pub stroke_color: Option<Color>,
    /// Custom shadow color (default: black).
    pub shadow_color: Option<Color>,
    /// Text shadow offset `(dx, dy)` in pixels. When set, text is drawn
    /// twice: first at `(x+dx, y+dy)` in `text_shadow_color`, then
    /// normally at `(x, y)`.
    pub text_shadow_offset: Option<(i32, i32)>,
    /// Text shadow color (default: black at 50% alpha).
    pub text_shadow_color: Option<Color>,

    // -- Accessibility --
    /// Human-readable label for screen readers.
    pub aria_label: Option<String>,
    /// Semantic role for assistive technology.
    pub role: Option<AccessibilityRole>,
}

impl SdiObject {
    /// Create a new object with sensible defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            alpha: 255,
            z: 0,
            visible: true,
            texture: None,
            color: Color::WHITE,
            text: None,
            font_size: 12,
            text_color: Color::BLACK,
            overlay: false,
            border_radius: None,
            gradient_top: None,
            gradient_bottom: None,
            shadow_level: None,
            stroke_width: None,
            stroke_color: None,
            shadow_color: None,
            text_shadow_offset: None,
            text_shadow_color: None,
            aria_label: None,
            role: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_object_defaults() {
        let obj = SdiObject::new("test");
        assert_eq!(obj.name, "test");
        assert_eq!(obj.x, 0);
        assert_eq!(obj.y, 0);
        assert_eq!(obj.alpha, 255);
        assert!(obj.visible);
        assert!(obj.texture.is_none());
    }

    #[test]
    fn new_accessibility_defaults_none() {
        let obj = SdiObject::new("btn");
        assert!(obj.aria_label.is_none());
        assert!(obj.role.is_none());
    }

    #[test]
    fn set_aria_label() {
        let mut obj = SdiObject::new("submit_btn");
        obj.aria_label = Some("Submit form".to_string());
        assert_eq!(obj.aria_label.as_deref(), Some("Submit form"));
    }

    #[test]
    fn set_role() {
        let mut obj = SdiObject::new("nav");
        obj.role = Some(AccessibilityRole::Navigation);
        assert_eq!(obj.role, Some(AccessibilityRole::Navigation));
    }

    #[test]
    fn role_equality() {
        assert_eq!(AccessibilityRole::Button, AccessibilityRole::Button);
        assert_ne!(AccessibilityRole::Button, AccessibilityRole::Heading);
    }

    #[test]
    fn role_clone() {
        let role = AccessibilityRole::TextField;
        let cloned = role;
        assert_eq!(role, cloned);
    }

    #[test]
    fn role_debug() {
        let role = AccessibilityRole::Status;
        let debug = format!("{role:?}");
        assert!(debug.contains("Status"));
    }

    #[test]
    fn object_default_color_is_white() {
        let obj = SdiObject::new("c");
        assert_eq!(obj.color, Color::WHITE);
    }

    #[test]
    fn object_default_text_color_is_black() {
        let obj = SdiObject::new("t");
        assert_eq!(obj.text_color, Color::BLACK);
    }

    #[test]
    fn object_default_font_size() {
        let obj = SdiObject::new("f");
        assert_eq!(obj.font_size, 12);
    }

    #[test]
    fn object_default_z_is_zero() {
        let obj = SdiObject::new("z");
        assert_eq!(obj.z, 0);
    }

    #[test]
    fn object_default_overlay_is_false() {
        let obj = SdiObject::new("o");
        assert!(!obj.overlay);
    }

    #[test]
    fn object_extended_props_default_none() {
        let obj = SdiObject::new("ext");
        assert!(obj.border_radius.is_none());
        assert!(obj.gradient_top.is_none());
        assert!(obj.gradient_bottom.is_none());
        assert!(obj.shadow_level.is_none());
        assert!(obj.stroke_width.is_none());
        assert!(obj.stroke_color.is_none());
        assert!(obj.shadow_color.is_none());
        assert!(obj.text_shadow_offset.is_none());
        assert!(obj.text_shadow_color.is_none());
    }

    #[test]
    fn object_clone() {
        let mut obj = SdiObject::new("clone_test");
        obj.x = 10;
        obj.aria_label = Some("label".to_string());
        obj.role = Some(AccessibilityRole::Button);
        let cloned = obj.clone();
        assert_eq!(cloned.name, "clone_test");
        assert_eq!(cloned.x, 10);
        assert_eq!(cloned.aria_label.as_deref(), Some("label"));
        assert_eq!(cloned.role, Some(AccessibilityRole::Button));
    }

    #[test]
    fn object_debug() {
        let obj = SdiObject::new("debug_test");
        let debug = format!("{obj:?}");
        assert!(debug.contains("debug_test"));
    }

    #[test]
    fn all_accessibility_roles_distinct() {
        let roles = [
            AccessibilityRole::Button,
            AccessibilityRole::Heading,
            AccessibilityRole::ListItem,
            AccessibilityRole::TextField,
            AccessibilityRole::MenuItem,
            AccessibilityRole::Image,
            AccessibilityRole::Group,
            AccessibilityRole::Status,
            AccessibilityRole::Navigation,
        ];
        for (i, a) in roles.iter().enumerate() {
            for (j, b) in roles.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
