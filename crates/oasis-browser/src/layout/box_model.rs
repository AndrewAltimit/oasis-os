//! Box model types for the layout engine.
//!
//! Defines rectangles, edge sizes, dimensions, box types, and the layout
//! tree data structures used by block and inline layout algorithms.

use crate::css::values::ComputedStyle;
use crate::html::dom::NodeId;
use oasis_types::backend::TextureId;

/// A rectangle with position and size.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Create a new rectangle.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Check if a point is inside this rectangle.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// Expand this rect to include another rect.
    ///
    /// Returns the smallest rectangle that contains both `self` and
    /// `other`. If either rect has zero area, the other is returned
    /// (with adjustments for position).
    pub fn union(&self, other: &Rect) -> Rect {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width).max(other.x + other.width);
        let y2 = (self.y + self.height).max(other.y + other.height);
        Rect {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1,
        }
    }
}

/// Edge sizes (top, right, bottom, left) used for margin, padding, border.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSizes {
    /// Create edge sizes with all four values.
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Create edge sizes with the same value on all sides.
    pub fn uniform(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// Total horizontal size (left + right).
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total vertical size (top + bottom).
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Full dimensions of a layout box.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dimensions {
    pub content: Rect,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub margin: EdgeSizes,
}

impl Dimensions {
    /// The padding box rect (content + padding).
    pub fn padding_box(&self) -> Rect {
        Rect {
            x: self.content.x - self.padding.left,
            y: self.content.y - self.padding.top,
            width: self.content.width + self.padding.left + self.padding.right,
            height: self.content.height + self.padding.top + self.padding.bottom,
        }
    }

    /// The border box rect (content + padding + border).
    pub fn border_box(&self) -> Rect {
        let pb = self.padding_box();
        Rect {
            x: pb.x - self.border.left,
            y: pb.y - self.border.top,
            width: pb.width + self.border.left + self.border.right,
            height: pb.height + self.border.top + self.border.bottom,
        }
    }

    /// The margin box rect (content + padding + border + margin).
    pub fn margin_box(&self) -> Rect {
        let bb = self.border_box();
        Rect {
            x: bb.x - self.margin.left,
            y: bb.y - self.margin.top,
            width: bb.width + self.margin.left + self.margin.right,
            height: bb.height + self.margin.top + self.margin.bottom,
        }
    }
}

/// The type of a layout box.
#[derive(Debug, Clone)]
pub enum BoxType {
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    TableWrapper,
    TableRow,
    TableCell,
    ListItem { marker: ListMarker },
    Replaced(ReplacedContent),
    Anonymous,
}

/// List item marker type.
#[derive(Debug, Clone)]
pub enum ListMarker {
    Disc,
    Circle,
    Square,
    /// The number to display for ordered lists.
    Decimal(usize),
    None,
}

/// Content for replaced elements (img, hr, br, input, button, select).
#[derive(Debug, Clone)]
pub enum ReplacedContent {
    Image {
        width: u32,
        height: u32,
        texture: Option<TextureId>,
        alt: String,
        /// Sub-region within an atlas texture. When `Some`, the texture
        /// is a shared atlas and rendering should use `blit_sub` with
        /// these source coordinates instead of blitting the full texture.
        atlas_region: Option<crate::image_atlas::AtlasRegion>,
    },
    HorizontalRule,
    LineBreak,
    /// A text input field.
    TextInput {
        value: String,
        placeholder: String,
        size: u32,
        is_password: bool,
    },
    /// A checkbox input.
    Checkbox {
        checked: bool,
    },
    /// A radio button input.
    RadioButton {
        checked: bool,
    },
    /// A multi-line text area.
    TextArea {
        value: String,
        placeholder: String,
        rows: u32,
        cols: u32,
    },
    /// A submit/button input.
    SubmitButton {
        label: String,
    },
    /// A `<select>` dropdown box.
    SelectBox {
        label: String,
    },
    /// An inline `<svg>` element.
    Svg {
        element: Box<crate::svg::SvgElement>,
    },
    /// A `<canvas>` element with a shared drawing state.
    Canvas {
        state: std::rc::Rc<std::cell::RefCell<crate::canvas::CanvasState>>,
    },
}

/// A single box in the layout tree.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub box_type: BoxType,
    pub dimensions: Dimensions,
    pub children: Vec<LayoutBox>,
    pub node: Option<NodeId>,
    pub style: ComputedStyle,
    /// Text content for inline leaf boxes representing DOM text nodes.
    pub text: Option<String>,
    /// Whether this box or any descendant needs relayout.
    pub dirty: bool,
    /// GPU texture for CSS `background-image`, assigned during the
    /// texture resolution pass.
    pub background_texture: Option<TextureId>,
}

impl LayoutBox {
    /// Create a new layout box with the given type, style, and DOM node.
    pub fn new(box_type: BoxType, style: ComputedStyle, node: Option<NodeId>) -> Self {
        Self {
            box_type,
            dimensions: Dimensions::default(),
            children: Vec::new(),
            node,
            style,
            text: None,
            dirty: true,
            background_texture: None,
        }
    }

    /// Returns true if this box is a block-level box.
    pub fn is_block_level(&self) -> bool {
        matches!(
            self.box_type,
            BoxType::Block
                | BoxType::Flex
                | BoxType::Grid
                | BoxType::ListItem { .. }
                | BoxType::TableWrapper
                | BoxType::Anonymous
                | BoxType::Replaced(ReplacedContent::HorizontalRule)
        )
    }

    /// Returns true if this box is inline-level.
    pub fn is_inline_level(&self) -> bool {
        matches!(self.box_type, BoxType::Inline | BoxType::InlineBlock)
    }

    /// Mark this box and all descendants as clean (layout complete).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
        for child in &mut self.children {
            child.mark_clean();
        }
    }

    /// Mark this box as needing relayout.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Recursively clear the dirty flag on this box and all descendants.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
        for child in &mut self.children {
            child.clear_dirty();
        }
    }

    /// Check whether this box or any descendant is dirty.
    pub fn any_dirty(&self) -> bool {
        if self.dirty {
            return true;
        }
        self.children.iter().any(|c| c.any_dirty())
    }

    /// Find the deepest DOM node at the given point. Returns `None`
    /// if the point is outside the layout tree or no box with a DOM
    /// node ID contains it.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        // If this box has CSS transforms, apply the inverse transform
        // to the test point so AABB check works in local coordinates.
        let (test_x, test_y) = if !self.style.transforms.is_empty() {
            let ox = self.dimensions.content.width / 2.0;
            let oy = self.dimensions.content.height / 2.0;
            let m = crate::transform::AffineTransform2D::from_css_transforms(
                &self.style.transforms,
                self.dimensions.content.x + ox,
                self.dimensions.content.y + oy,
            );
            if let Some(inv) = m.inverse() {
                inv.apply(x, y)
            } else {
                (x, y) // singular matrix — fall back to untransformed
            }
        } else {
            (x, y)
        };

        let d = &self.dimensions;
        let bx = d.content.x - d.padding.left - d.border.left;
        let by = d.content.y - d.padding.top - d.border.top;
        let bw =
            d.content.width + d.padding.left + d.padding.right + d.border.left + d.border.right;
        let bh =
            d.content.height + d.padding.top + d.padding.bottom + d.border.top + d.border.bottom;

        if test_x < bx || test_x >= bx + bw || test_y < by || test_y >= by + bh {
            return None;
        }

        // Check children deepest-first (later children paint on top).
        for child in self.children.iter().rev() {
            if let Some(nid) = child.hit_test(x, y) {
                return Some(nid);
            }
        }

        self.node
    }

    /// Mark a specific node and its ancestors as dirty.
    pub fn mark_node_dirty(&mut self, target_node: NodeId) -> bool {
        if self.node == Some(target_node) {
            self.dirty = true;
            return true;
        }
        for child in &mut self.children {
            if child.mark_node_dirty(target_node) {
                self.dirty = true;
                return true;
            }
        }
        false
    }
}

/// A line box containing inline fragments.
#[derive(Debug, Clone)]
pub struct LineBox {
    pub fragments: Vec<InlineFragment>,
    pub baseline: f32,
    pub height: f32,
    pub width: f32,
}

impl LineBox {
    /// Create a new empty line box with the given available width.
    pub fn new(available_width: f32) -> Self {
        Self {
            fragments: Vec::new(),
            baseline: 0.0,
            height: 0.0,
            width: available_width,
        }
    }

    /// The current used width of this line (sum of fragment widths).
    pub fn used_width(&self) -> f32 {
        self.fragments.iter().map(InlineFragment::width).sum()
    }

    /// Returns true if this line box has no fragments.
    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    /// Try to add a fragment to this line. Returns `true` if it fits,
    /// `false` if the line is full and a new line should be started.
    pub fn try_add(&mut self, fragment: &InlineFragment) -> bool {
        let frag_width = fragment.width();
        if !self.fragments.is_empty() && self.used_width() + frag_width > self.width {
            return false;
        }
        self.fragments.push(fragment.clone());
        true
    }
}

/// A fragment of inline content within a line box.
#[derive(Debug, Clone)]
pub enum InlineFragment {
    Text {
        text: String,
        x: f32,
        width: f32,
        style: ComputedStyle,
        node: Option<NodeId>,
    },
    InlineBox {
        layout_box: LayoutBox,
    },
    ReplacedInline {
        replaced: ReplacedContent,
        x: f32,
        width: f32,
        height: f32,
        style: ComputedStyle,
        node: Option<NodeId>,
    },
}

impl InlineFragment {
    /// The width of this fragment.
    pub fn width(&self) -> f32 {
        match self {
            InlineFragment::Text { width, .. } => *width,
            InlineFragment::InlineBox { layout_box } => layout_box.dimensions.margin_box().width,
            InlineFragment::ReplacedInline { width, .. } => *width,
        }
    }

    /// The height of this fragment (line-height for text, border-box
    /// for inline boxes).
    pub fn height(&self) -> f32 {
        match self {
            InlineFragment::Text { style, .. } => style.line_height,
            InlineFragment::InlineBox { layout_box } => layout_box.dimensions.margin_box().height,
            InlineFragment::ReplacedInline { height, .. } => *height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        // Inside
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(50.0, 40.0));
        assert!(r.contains(109.9, 69.9));
        // Outside
        assert!(!r.contains(9.9, 20.0));
        assert!(!r.contains(10.0, 19.9));
        assert!(!r.contains(110.0, 20.0));
        assert!(!r.contains(10.0, 70.0));
    }

    #[test]
    fn rect_contains_zero_size() {
        let r = Rect::new(5.0, 5.0, 0.0, 0.0);
        assert!(!r.contains(5.0, 5.0));
    }

    #[test]
    fn dimensions_padding_box() {
        let d = Dimensions {
            content: Rect::new(20.0, 30.0, 100.0, 50.0),
            padding: EdgeSizes::new(5.0, 10.0, 5.0, 10.0),
            border: EdgeSizes::default(),
            margin: EdgeSizes::default(),
        };
        let pb = d.padding_box();
        assert_eq!(pb.x, 10.0); // 20 - 10
        assert_eq!(pb.y, 25.0); // 30 - 5
        assert_eq!(pb.width, 120.0); // 100 + 10 + 10
        assert_eq!(pb.height, 60.0); // 50 + 5 + 5
    }

    #[test]
    fn dimensions_border_box() {
        let d = Dimensions {
            content: Rect::new(30.0, 30.0, 100.0, 50.0),
            padding: EdgeSizes::new(5.0, 5.0, 5.0, 5.0),
            border: EdgeSizes::new(2.0, 2.0, 2.0, 2.0),
            margin: EdgeSizes::default(),
        };
        let bb = d.border_box();
        assert_eq!(bb.x, 23.0); // 30 - 5 - 2
        assert_eq!(bb.y, 23.0); // 30 - 5 - 2
        assert_eq!(bb.width, 114.0); // 100 + 10 + 4
        assert_eq!(bb.height, 64.0); // 50 + 10 + 4
    }

    #[test]
    fn dimensions_margin_box() {
        let d = Dimensions {
            content: Rect::new(50.0, 50.0, 100.0, 40.0),
            padding: EdgeSizes::new(5.0, 5.0, 5.0, 5.0),
            border: EdgeSizes::new(1.0, 1.0, 1.0, 1.0),
            margin: EdgeSizes::new(10.0, 10.0, 10.0, 10.0),
        };
        let mb = d.margin_box();
        // padding_box.x = 50 - 5 = 45
        // border_box.x  = 45 - 1 = 44
        // margin_box.x  = 44 - 10 = 34
        assert_eq!(mb.x, 34.0);
        assert_eq!(mb.y, 34.0);
        // padding_box.width = 100 + 5 + 5 = 110
        // border_box.width  = 110 + 1 + 1 = 112
        // margin_box.width  = 112 + 10 + 10 = 132
        assert_eq!(mb.width, 132.0);
        // padding_box.height = 40 + 5 + 5 = 50
        // border_box.height  = 50 + 1 + 1 = 52
        // margin_box.height  = 52 + 10 + 10 = 72
        assert_eq!(mb.height, 72.0);
    }

    #[test]
    fn edge_sizes_default_is_zero() {
        let e = EdgeSizes::default();
        assert_eq!(e.top, 0.0);
        assert_eq!(e.right, 0.0);
        assert_eq!(e.bottom, 0.0);
        assert_eq!(e.left, 0.0);
    }

    #[test]
    fn edge_sizes_horizontal_vertical() {
        let e = EdgeSizes::new(3.0, 7.0, 4.0, 6.0);
        assert_eq!(e.horizontal(), 13.0);
        assert_eq!(e.vertical(), 7.0);
    }

    #[test]
    fn rect_union() {
        let a = Rect::new(10.0, 20.0, 30.0, 40.0);
        let b = Rect::new(5.0, 25.0, 50.0, 10.0);
        let u = a.union(&b);
        assert_eq!(u.x, 5.0);
        assert_eq!(u.y, 20.0);
        // right edge: max(10+30=40, 5+50=55) = 55; width = 55 - 5 = 50
        assert_eq!(u.width, 50.0);
        // bottom edge: max(20+40=60, 25+10=35) = 60; height = 60 - 20 = 40
        assert_eq!(u.height, 40.0);
    }

    #[test]
    fn rect_union_same_rect() {
        let r = Rect::new(10.0, 10.0, 50.0, 50.0);
        let u = r.union(&r);
        assert_eq!(u, r);
    }

    #[test]
    fn layout_box_constructor() {
        let style = ComputedStyle::default();
        let lb = LayoutBox::new(BoxType::Block, style.clone(), Some(42));
        assert!(lb.is_block_level());
        assert!(!lb.is_inline_level());
        assert!(lb.children.is_empty());
        assert_eq!(lb.node, Some(42));
    }

    #[test]
    fn line_box_try_add() {
        let style = ComputedStyle::default();
        let mut line = LineBox::new(100.0);
        assert!(line.is_empty());

        let frag1 = InlineFragment::Text {
            text: "Hello".into(),
            x: 0.0,
            width: 40.0,
            style: style.clone(),
            node: None,
        };
        assert!(line.try_add(&frag1));
        assert_eq!(line.used_width(), 40.0);

        let frag2 = InlineFragment::Text {
            text: "World".into(),
            x: 0.0,
            width: 40.0,
            style: style.clone(),
            node: None,
        };
        assert!(line.try_add(&frag2));
        assert_eq!(line.used_width(), 80.0);

        // This one should not fit (80 + 30 > 100).
        let frag3 = InlineFragment::Text {
            text: "!".into(),
            x: 0.0,
            width: 30.0,
            style,
            node: None,
        };
        assert!(!line.try_add(&frag3));
    }

    #[test]
    fn line_box_first_fragment_always_fits() {
        let style = ComputedStyle::default();
        let mut line = LineBox::new(50.0);
        // Even if wider than the line, the first fragment always fits
        // to avoid infinite loops.
        let frag = InlineFragment::Text {
            text: "VeryLongWord".into(),
            x: 0.0,
            width: 200.0,
            style,
            node: None,
        };
        assert!(line.try_add(&frag));
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        fn arb_edge() -> impl Strategy<Value = EdgeSizes> {
            (0.0f32..100.0, 0.0f32..100.0, 0.0f32..100.0, 0.0f32..100.0)
                .prop_map(|(t, r, b, l)| EdgeSizes::new(t, r, b, l))
        }

        fn arb_rect() -> impl Strategy<Value = Rect> {
            (
                -500.0f32..500.0,
                -500.0f32..500.0,
                0.0f32..500.0,
                0.0f32..500.0,
            )
                .prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
        }

        fn arb_dimensions() -> impl Strategy<Value = Dimensions> {
            (arb_rect(), arb_edge(), arb_edge(), arb_edge()).prop_map(
                |(content, padding, border, margin)| Dimensions {
                    content,
                    padding,
                    border,
                    margin,
                },
            )
        }

        proptest! {
            #[test]
            fn padding_box_width_equals_content_plus_padding(d in arb_dimensions()) {
                let pb = d.padding_box();
                let expected_w = d.content.width + d.padding.left + d.padding.right;
                prop_assert!(
                    (pb.width - expected_w).abs() < 0.001,
                    "padding_box width: got {}, expected {expected_w}", pb.width
                );
            }

            #[test]
            fn border_box_width_equals_content_plus_padding_plus_border(d in arb_dimensions()) {
                let bb = d.border_box();
                let expected_w = d.content.width
                    + d.padding.left + d.padding.right
                    + d.border.left + d.border.right;
                prop_assert!(
                    (bb.width - expected_w).abs() < 0.01,
                    "border_box width: got {}, expected {expected_w}", bb.width
                );
            }

            #[test]
            fn margin_box_width_equals_total(d in arb_dimensions()) {
                let mb = d.margin_box();
                let expected_w = d.content.width
                    + d.padding.horizontal()
                    + d.border.horizontal()
                    + d.margin.horizontal();
                prop_assert!(
                    (mb.width - expected_w).abs() < 0.01,
                    "margin_box width: got {}, expected {expected_w}", mb.width
                );
            }

            #[test]
            fn margin_box_height_equals_total(d in arb_dimensions()) {
                let mb = d.margin_box();
                let expected_h = d.content.height
                    + d.padding.vertical()
                    + d.border.vertical()
                    + d.margin.vertical();
                prop_assert!(
                    (mb.height - expected_h).abs() < 0.01,
                    "margin_box height: got {}, expected {expected_h}", mb.height
                );
            }

            #[test]
            fn boxes_nest_correctly(d in arb_dimensions()) {
                let pb = d.padding_box();
                let bb = d.border_box();
                let mb = d.margin_box();
                // Each layer's width >= the inner layer's width.
                prop_assert!(pb.width >= d.content.width - 0.001);
                prop_assert!(bb.width >= pb.width - 0.001);
                prop_assert!(mb.width >= bb.width - 0.001);
                prop_assert!(pb.height >= d.content.height - 0.001);
                prop_assert!(bb.height >= pb.height - 0.001);
                prop_assert!(mb.height >= bb.height - 0.001);
            }

            #[test]
            fn edge_sizes_horizontal_is_left_plus_right(
                t in 0.0f32..100.0, r in 0.0f32..100.0,
                b in 0.0f32..100.0, l in 0.0f32..100.0,
            ) {
                let e = EdgeSizes::new(t, r, b, l);
                prop_assert!((e.horizontal() - (l + r)).abs() < 0.001);
                prop_assert!((e.vertical() - (t + b)).abs() < 0.001);
            }

            #[test]
            fn uniform_edge_all_equal(v in 0.0f32..100.0) {
                let e = EdgeSizes::uniform(v);
                prop_assert!((e.top - v).abs() < 0.001);
                prop_assert!((e.right - v).abs() < 0.001);
                prop_assert!((e.bottom - v).abs() < 0.001);
                prop_assert!((e.left - v).abs() < 0.001);
            }

            #[test]
            fn rect_union_is_commutative(a in arb_rect(), b in arb_rect()) {
                let u1 = a.union(&b);
                let u2 = b.union(&a);
                prop_assert!((u1.x - u2.x).abs() < 0.001);
                prop_assert!((u1.y - u2.y).abs() < 0.001);
                prop_assert!((u1.width - u2.width).abs() < 0.001);
                prop_assert!((u1.height - u2.height).abs() < 0.001);
            }

            #[test]
            fn rect_union_contains_both(a in arb_rect(), b in arb_rect()) {
                let u = a.union(&b);
                // Union x,y should be <= both inputs' x,y.
                prop_assert!(u.x <= a.x + 0.001);
                prop_assert!(u.x <= b.x + 0.001);
                prop_assert!(u.y <= a.y + 0.001);
                prop_assert!(u.y <= b.y + 0.001);
                // Union right/bottom edge should be >= both inputs'.
                prop_assert!(u.x + u.width >= a.x + a.width - 0.001);
                prop_assert!(u.x + u.width >= b.x + b.width - 0.001);
                prop_assert!(u.y + u.height >= a.y + a.height - 0.001);
                prop_assert!(u.y + u.height >= b.y + b.height - 0.001);
            }

            #[test]
            fn rect_union_with_self_is_identity(r in arb_rect()) {
                let u = r.union(&r);
                prop_assert!((u.x - r.x).abs() < 0.001);
                prop_assert!((u.y - r.y).abs() < 0.001);
                prop_assert!((u.width - r.width).abs() < 0.001);
                prop_assert!((u.height - r.height).abs() < 0.001);
            }

            #[test]
            fn rect_contains_interior_point(
                x in -500.0f32..500.0, y in -500.0f32..500.0,
                w in 1.0f32..500.0, h in 1.0f32..500.0,
            ) {
                let r = Rect::new(x, y, w, h);
                // Midpoint should always be contained.
                let mid_x = x + w / 2.0;
                let mid_y = y + h / 2.0;
                prop_assert!(r.contains(mid_x, mid_y));
            }

            /// Content width clamped between min and max produces
            /// a valid box where min_w <= content.width <= max_w.
            #[test]
            fn content_width_clamped_by_min_max(
                raw_w in 0.0f32..500.0,
                min_w in 0.0f32..250.0,
                max_w in 250.0f32..500.0,
            ) {
                let clamped = raw_w.clamp(min_w, max_w);
                prop_assert!(clamped >= min_w - 0.001,
                    "clamped {} < min {}", clamped, min_w);
                prop_assert!(clamped <= max_w + 0.001,
                    "clamped {} > max {}", clamped, max_w);
            }

            /// margin + padding + border + content == margin_box total
            /// (verifying the additive property of all box layers).
            #[test]
            fn all_layers_add_up(d in arb_dimensions()) {
                let mb = d.margin_box();
                let total_w = d.content.width
                    + d.padding.horizontal()
                    + d.border.horizontal()
                    + d.margin.horizontal();
                let total_h = d.content.height
                    + d.padding.vertical()
                    + d.border.vertical()
                    + d.margin.vertical();
                prop_assert!(
                    (mb.width - total_w).abs() < 0.01,
                    "width: margin_box={}, sum={total_w}", mb.width
                );
                prop_assert!(
                    (mb.height - total_h).abs() < 0.01,
                    "height: margin_box={}, sum={total_h}", mb.height
                );
            }

            /// padding_box position is offset from content by padding.
            #[test]
            fn padding_box_position_offset(d in arb_dimensions()) {
                let pb = d.padding_box();
                prop_assert!(
                    (pb.x - (d.content.x - d.padding.left)).abs() < 0.001,
                    "pb.x={}, expected={}",
                    pb.x, d.content.x - d.padding.left
                );
                prop_assert!(
                    (pb.y - (d.content.y - d.padding.top)).abs() < 0.001,
                    "pb.y={}, expected={}",
                    pb.y, d.content.y - d.padding.top
                );
            }

            /// Rect::contains returns false for points just outside
            /// each edge.
            #[test]
            fn rect_does_not_contain_outside_points(
                x in -500.0f32..500.0, y in -500.0f32..500.0,
                w in 1.0f32..500.0, h in 1.0f32..500.0,
            ) {
                let r = Rect::new(x, y, w, h);
                // Just left of the rect.
                prop_assert!(!r.contains(x - 0.01, y + h / 2.0));
                // Just above the rect.
                prop_assert!(!r.contains(x + w / 2.0, y - 0.01));
                // Just right of the rect (at x + w, exclusive).
                prop_assert!(!r.contains(x + w, y + h / 2.0));
                // Just below the rect (at y + h, exclusive).
                prop_assert!(!r.contains(x + w / 2.0, y + h));
            }
        }

        #[test]
        fn layout_box_dirty_tracking() {
            let style = ComputedStyle::default();
            let mut root = LayoutBox::new(BoxType::Block, style.clone(), Some(0));
            let mut child = LayoutBox::new(BoxType::Block, style.clone(), Some(1));
            child.dirty = false;
            root.children.push(child);

            // Root is dirty, so any_dirty returns true.
            assert!(root.any_dirty());

            // Clear all dirty flags.
            root.clear_dirty();
            assert!(!root.any_dirty());
            assert!(!root.dirty);
            assert!(!root.children[0].dirty);

            // Mark a specific node dirty.
            root.mark_node_dirty(1);
            assert!(root.dirty);
            assert!(root.children[0].dirty);
        }

        #[test]
        fn mark_node_dirty_missing_node() {
            let style = ComputedStyle::default();
            let mut root = LayoutBox::new(BoxType::Block, style, Some(0));
            root.dirty = false;
            // Marking a non-existent node returns false.
            assert!(!root.mark_node_dirty(99));
            assert!(!root.dirty);
        }
    }
}
