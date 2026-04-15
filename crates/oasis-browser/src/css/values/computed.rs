//! `ComputedStyle` struct definition, `Default` impl, and inheritance.

use rustc_hash::FxHashMap;

use oasis_types::backend::Color;

use super::types::{
    AlignContent, AlignItems, AlignSelf, Animation, Appearance, BackfaceVisibility, BackgroundBox,
    BackgroundImage, BackgroundPosition, BackgroundRepeat, BackgroundSize, BlendMode,
    BorderCollapse, BorderRadius, BorderStyle, BoxShadow, BoxSizing, Clear, ClipPath, ColorScheme,
    ContainerType, ContentVisibility, Cursor, Dimension, Display, FieldSizing, FilterFunction,
    FlexDirection, FlexWrap, Float, FontFamily, FontKerning, FontStretch, FontStyle, FontVariant,
    FontWeight, GridTrackSize, Hyphens, ImageRendering, Isolation, JustifyContent, JustifySelf,
    ListStylePosition, ListStyleType, ObjectFit, ObjectPosition, Overflow, OverflowWrap,
    OverscrollBehavior, PointerEvents, Position, ROOT_FONT_SIZE, Resize, ScrollBehavior,
    ScrollSnapAlign, ScrollSnapStop, TextAlign, TextAlignLast, TextDecoration, TextDirection,
    TextJustify, TextOverflow, TextRendering, TextShadow, TextTransform, TextUnderlinePosition,
    TextWrap, TouchAction, TransformOrigin, TransformStyle, Transition, UserSelect, VerticalAlign,
    Visibility, WhiteSpace, WordBreak,
};

/// Computed style for a DOM node after cascade resolution.
///
/// All lengths are resolved to absolute pixels. Relative units (em, %)
/// have been converted during property application. Inherited properties
/// that were not explicitly set carry the parent's computed value.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    // -- Display ----------------------------------------------------
    pub display: Display,
    pub visibility: Visibility,

    // -- Box model --------------------------------------------------
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_top_width: f32,
    pub border_right_width: f32,
    pub border_bottom_width: f32,
    pub border_left_width: f32,
    pub border_top_color: Color,
    pub border_right_color: Color,
    pub border_bottom_color: Color,
    pub border_left_color: Color,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,

    // -- Dimensions -------------------------------------------------
    pub width: Dimension,
    pub height: Dimension,
    pub max_width: Dimension,
    pub min_width: Dimension,
    pub max_height: Dimension,
    pub min_height: Dimension,

    // -- Text -------------------------------------------------------
    pub color: Color,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_family: FontFamily,
    pub text_align: TextAlign,
    pub direction: TextDirection,
    pub text_decoration: TextDecoration,
    pub text_indent: f32,
    pub text_transform: TextTransform,
    pub line_height: f32,
    /// Unitless line-height factor (e.g. 1.5 from `line-height: 1.5`).
    /// When present, inherited line-height is recomputed as
    /// `factor * child_font_size` rather than copying the parent's
    /// computed pixel value. Per CSS 2.1 §17.21.
    pub line_height_factor: Option<f32>,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub white_space: WhiteSpace,
    /// CSS `text-wrap` property. Stored but not yet applied during
    /// line breaking — `Balance` / `Pretty` / `Stable` currently fall
    /// through to the default `Wrap` behaviour.
    pub text_wrap: TextWrap,

    // -- Background -------------------------------------------------
    pub background_color: Color,

    // -- List -------------------------------------------------------
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,

    // -- Table ------------------------------------------------------
    pub border_collapse: BorderCollapse,
    pub border_spacing: f32,

    // -- Float ------------------------------------------------------
    pub float: Float,
    pub clear: Clear,

    // -- Overflow ---------------------------------------------------
    pub overflow: Overflow,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,

    // -- Positioning ------------------------------------------------
    pub position: Position,
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,
    pub z_index: i32,
    /// True when `z-index` was not explicitly set (CSS initial value `auto`).
    /// `z-index: auto` does NOT create a stacking context for positioned
    /// elements, whereas `z-index: 0` (explicitly set) does.
    pub z_index_auto: bool,

    // -- Replaced element sizing ----------------------------------------
    pub object_fit: ObjectFit,
    pub object_position: ObjectPosition,

    // -- Interaction ---------------------------------------------------
    pub cursor: Cursor,
    pub pointer_events: PointerEvents,
    pub user_select: UserSelect,

    // -- Aspect ratio --------------------------------------------------
    pub aspect_ratio: Option<f32>,

    // -- Text underline offset -----------------------------------------
    pub text_underline_offset: f32,

    // -- Visual effects -----------------------------------------------
    pub border_radius: BorderRadius,
    pub box_shadow: Vec<BoxShadow>,
    pub text_shadow: Option<TextShadow>,
    pub opacity: f32,

    // -- Outline ----------------------------------------------------------
    pub outline_width: f32,
    pub outline_color: Color,
    pub outline_style: BorderStyle,
    pub outline_offset: f32,

    // -- Box sizing -----------------------------------------------------
    pub box_sizing: BoxSizing,

    // -- Text overflow --------------------------------------------------
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub text_overflow: TextOverflow,

    // -- Vertical alignment ---------------------------------------------
    pub vertical_align: VerticalAlign,

    // -- Background image -----------------------------------------------
    pub background_image: BackgroundImage,
    pub background_size: BackgroundSize,
    pub background_position: BackgroundPosition,
    pub background_repeat: BackgroundRepeat,

    // -- Generated content (::before/::after) ---------------------------
    pub content: Option<String>,
    pub before_content: Option<String>,
    pub after_content: Option<String>,

    pub before_style: Option<Box<ComputedStyle>>,
    pub after_style: Option<Box<ComputedStyle>>,

    // -- Margin auto flags (for block centering) -------------------------
    pub margin_left_auto: bool,
    pub margin_right_auto: bool,
    pub margin_top_auto: bool,
    pub margin_bottom_auto: bool,

    // -- Flexbox properties --
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_content: AlignContent,
    pub align_self: AlignSelf,
    pub order: i32,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dimension,
    pub gap: f32,

    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    pub grid_column_start: Option<i32>,
    pub grid_column_end: Option<i32>,
    pub grid_row_start: Option<i32>,
    pub grid_row_end: Option<i32>,
    pub column_gap: f32,
    pub row_gap: f32,

    // -- Percentage padding/margin (resolved against containing width) ---
    /// When `Some(pct)`, padding-top was specified as a percentage.
    pub padding_top_pct: Option<f32>,
    pub padding_right_pct: Option<f32>,
    pub padding_bottom_pct: Option<f32>,
    pub padding_left_pct: Option<f32>,
    pub margin_top_pct: Option<f32>,
    pub margin_right_pct: Option<f32>,
    pub margin_bottom_pct: Option<f32>,
    pub margin_left_pct: Option<f32>,

    // -- Transforms ---------------------------------------------------
    pub transforms: Vec<super::types::TransformFunction>,
    pub transform_origin: Option<TransformOrigin>,

    // -- Filters -------------------------------------------------------
    pub filters: Vec<FilterFunction>,

    // -- Counters -------------------------------------------------------
    pub counter_reset: Vec<(String, i32)>,
    pub counter_increment: Vec<(String, i32)>,

    // -- Will-change ---------------------------------------------------
    /// `will-change` declared a hint that promotes this element to its
    /// own compositing layer (and thus its own stacking context). True
    /// for any of `transform`, `opacity`, `filter`, `scroll-position`,
    /// or `contents`. The compositor inspects this flag in
    /// `creates_compositing_layer` / `creates_stacking_context`.
    pub will_change_promotes_layer: bool,

    // -- Tab size (for preformatted text) --------------------------------
    pub tab_size: u32,

    // -- Multi-column ---------------------------------------------------
    pub column_count: u32,
    pub column_width: f32,

    // -- Grid extensions ------------------------------------------------
    pub grid_auto_flow_column: bool,
    pub grid_template_areas: Vec<Vec<String>>,
    pub grid_area: Option<String>,
    pub grid_auto_rows: Vec<GridTrackSize>,
    pub grid_auto_columns: Vec<GridTrackSize>,

    // -- Table extensions -----------------------------------------------
    pub table_layout_fixed: bool,

    // -- Transitions ---------------------------------------------------
    pub transitions: Vec<Transition>,

    // -- Animations ---------------------------------------------------
    pub animations: Vec<Animation>,

    // -- Appearance ----------------------------------------------------
    pub appearance: Appearance,

    // -- Line clamp ----------------------------------------------------
    /// Number of lines to clamp to, or 0 for no clamping.
    pub line_clamp: u32,

    // -- Accent / caret colors -----------------------------------------
    pub accent_color: Option<Color>,
    pub caret_color: Option<Color>,

    // -- Color scheme --------------------------------------------------
    pub color_scheme: ColorScheme,

    // -- Isolation -----------------------------------------------------
    pub isolation: Isolation,

    // -- Resize --------------------------------------------------------
    pub resize: Resize,

    // -- Touch action --------------------------------------------------
    pub touch_action: TouchAction,

    // -- Scroll/snap/overscroll ----------------------------------------
    pub scroll_behavior: ScrollBehavior,
    pub scroll_snap_align: ScrollSnapAlign,
    pub scroll_snap_stop: ScrollSnapStop,
    /// Raw `scroll-snap-type` value (e.g. "x mandatory"). Stored opaque.
    pub scroll_snap_type: Option<String>,
    pub overscroll_behavior_x: OverscrollBehavior,
    pub overscroll_behavior_y: OverscrollBehavior,

    // -- Compositing ---------------------------------------------------
    pub mix_blend_mode: BlendMode,
    pub background_blend_mode: BlendMode,
    pub backdrop_filters: Vec<FilterFunction>,
    pub background_clip: BackgroundBox,
    pub background_origin: BackgroundBox,
    pub image_rendering: ImageRendering,
    pub content_visibility: ContentVisibility,

    // -- Mask properties (compositor overhaul PR6) ---------------------
    /// `mask-image` URL (`url(...)`) parsed as a raw string. None = no mask.
    pub mask_image: Option<String>,
    /// `mask-mode` — alpha / luminance / match-source.
    pub mask_mode: crate::css::values::types::MaskMode,
    /// `mask-composite` (single-layer). Multi-layer composition falls
    /// back to `Add` until the compositor grows multi-layer support.
    pub mask_composite: crate::css::values::types::MaskComposite,
    /// `mask-clip` — which box is the mask clipped to.
    pub mask_clip: BackgroundBox,
    /// `mask-origin` — which box is the mask positioned relative to.
    pub mask_origin: BackgroundBox,
    /// `mask-position` — reused from the background-position machinery.
    pub mask_position: BackgroundPosition,
    /// `mask-size` — reused from background-size.
    pub mask_size: BackgroundSize,
    /// `mask-repeat` — reused from background-repeat.
    pub mask_repeat: BackgroundRepeat,

    // -- Font extensions -----------------------------------------------
    pub font_variant: FontVariant,
    pub font_stretch: FontStretch,
    pub font_kerning: FontKerning,
    /// Raw `font-feature-settings` (e.g. `"liga" on, "kern" off`). Stored opaque.
    pub font_feature_settings: Option<String>,

    // -- Text extensions -----------------------------------------------
    pub hyphens: Hyphens,
    pub text_align_last: TextAlignLast,
    pub text_justify: TextJustify,
    pub text_underline_position: TextUnderlinePosition,
    /// `text-decoration-thickness`. `None` = `auto`.
    pub text_decoration_thickness: Option<f32>,
    pub text_rendering: TextRendering,

    // -- 3D / clipping -------------------------------------------------
    /// Structured `clip-path` value. `None` means `clip-path: none`.
    pub clip_path: Option<ClipPath>,
    pub perspective: Option<f32>,
    /// Raw `perspective-origin` value (e.g. `center`, `50% 50%`).
    pub perspective_origin: Option<String>,
    pub backface_visibility: BackfaceVisibility,
    pub transform_style: TransformStyle,

    // -- Grid alignment extensions -------------------------------------
    pub justify_self: JustifySelf,
    pub justify_items: JustifySelf,

    // -- CSS custom properties (--*) ------------------------------------
    pub custom_properties: FxHashMap<String, String>,

    // -- Container queries ----------------------------------------------
    /// `container-type`. Default `Normal`. When set to `InlineSize` or
    /// `Size`, this element establishes a query container that
    /// descendant `@container` rules can target.
    pub container_type: ContainerType,
    /// `container-name`. Zero or more identifiers; an `@container name (...)`
    /// rule must match one of these names on an ancestor for its rules
    /// to apply. Empty means the container is unnamed and only matches
    /// nameless `@container (...)` queries.
    pub container_name: Vec<String>,

    // -- Field sizing ---------------------------------------------------
    /// `field-sizing` property for form controls. Default `Fixed`
    /// keeps the input at its CSS / `size` attribute width;
    /// `Content` shrinks/grows to fit the current value.
    pub field_sizing: FieldSizing,
}

/// Standard browser defaults (CSS 2.1 initial values).
impl Default for ComputedStyle {
    fn default() -> Self {
        let base_font_size: f32 = ROOT_FONT_SIZE;
        Self {
            // Display
            display: Display::Inline,
            visibility: Visibility::Visible,

            // Box model -- all zero
            margin_top: 0.0,
            margin_right: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_top_width: 0.0,
            border_right_width: 0.0,
            border_bottom_width: 0.0,
            border_left_width: 0.0,
            border_top_color: Color::BLACK,
            border_right_color: Color::BLACK,
            border_bottom_color: Color::BLACK,
            border_left_color: Color::BLACK,
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,

            // Dimensions
            width: Dimension::Auto,
            height: Dimension::Auto,
            max_width: Dimension::Auto,
            min_width: Dimension::Px(0.0),
            max_height: Dimension::Auto,
            min_height: Dimension::Auto,

            // Text
            color: Color::BLACK,
            font_size: base_font_size,
            font_weight: FontWeight::NORMAL,
            font_style: FontStyle::Normal,
            font_family: FontFamily::SansSerif,
            text_align: TextAlign::Left,
            direction: TextDirection::Ltr,
            text_decoration: TextDecoration::NONE,
            text_indent: 0.0,
            text_transform: TextTransform::None,
            line_height: base_font_size * 1.5,
            line_height_factor: Some(1.5),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            white_space: WhiteSpace::Normal,
            text_wrap: TextWrap::Wrap,

            // Background -- transparent
            background_color: Color::rgba(0, 0, 0, 0),

            // List
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,

            // Table
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,

            // Float
            float: Float::None,
            clear: Clear::None,

            // Overflow
            overflow: Overflow::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,

            // Positioning
            position: Position::Static,
            top: Dimension::Auto,
            right: Dimension::Auto,
            bottom: Dimension::Auto,
            left: Dimension::Auto,
            z_index: 0,
            z_index_auto: true,

            // Replaced element sizing
            object_fit: ObjectFit::Fill,
            object_position: ObjectPosition::default(),

            // Interaction
            cursor: Cursor::Auto,
            pointer_events: PointerEvents::Auto,
            user_select: UserSelect::Auto,

            // Aspect ratio
            aspect_ratio: None,

            // Text underline offset
            text_underline_offset: 0.0,

            // Visual effects
            border_radius: BorderRadius::ZERO,
            box_shadow: Vec::new(),
            text_shadow: None,
            opacity: 1.0,

            // Outline
            outline_width: 0.0,
            outline_color: Color::BLACK,
            outline_style: BorderStyle::None,
            outline_offset: 0.0,

            // Box sizing
            box_sizing: BoxSizing::ContentBox,

            // Text overflow
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            text_overflow: TextOverflow::Clip,

            // Vertical alignment
            vertical_align: VerticalAlign::Baseline,

            // Background image
            background_image: BackgroundImage::None,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,

            // Generated content
            content: None,
            before_content: None,
            after_content: None,

            before_style: None,
            after_style: None,

            margin_left_auto: false,
            margin_right_auto: false,
            margin_top_auto: false,
            margin_bottom_auto: false,

            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_content: AlignContent::Stretch,
            align_self: AlignSelf::Auto,
            order: 0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dimension::Auto,
            gap: 0.0,

            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_column_start: None,
            grid_column_end: None,
            grid_row_start: None,
            grid_row_end: None,
            column_gap: 0.0,
            row_gap: 0.0,

            padding_top_pct: None,
            padding_right_pct: None,
            padding_bottom_pct: None,
            padding_left_pct: None,
            margin_top_pct: None,
            margin_right_pct: None,
            margin_bottom_pct: None,
            margin_left_pct: None,

            transforms: Vec::new(),
            transform_origin: None,

            filters: Vec::new(),

            counter_reset: Vec::new(),
            counter_increment: Vec::new(),

            will_change_promotes_layer: false,

            tab_size: 8,

            column_count: 0,
            column_width: 0.0,

            grid_auto_flow_column: false,
            grid_template_areas: Vec::new(),
            grid_area: None,
            grid_auto_rows: Vec::new(),
            grid_auto_columns: Vec::new(),

            table_layout_fixed: false,

            transitions: Vec::new(),

            animations: Vec::new(),

            appearance: Appearance::Auto,
            line_clamp: 0,
            accent_color: None,
            caret_color: None,
            color_scheme: ColorScheme::Normal,
            isolation: Isolation::Auto,
            resize: Resize::None,
            touch_action: TouchAction::Auto,

            scroll_behavior: ScrollBehavior::Auto,
            scroll_snap_align: ScrollSnapAlign::None,
            scroll_snap_stop: ScrollSnapStop::Normal,
            scroll_snap_type: None,
            overscroll_behavior_x: OverscrollBehavior::Auto,
            overscroll_behavior_y: OverscrollBehavior::Auto,

            mix_blend_mode: BlendMode::Normal,
            background_blend_mode: BlendMode::Normal,
            backdrop_filters: Vec::new(),
            background_clip: BackgroundBox::BorderBox,
            background_origin: BackgroundBox::PaddingBox,
            image_rendering: ImageRendering::Auto,
            content_visibility: ContentVisibility::Visible,

            mask_image: None,
            mask_mode: crate::css::values::types::MaskMode::MatchSource,
            mask_composite: crate::css::values::types::MaskComposite::Add,
            mask_clip: BackgroundBox::BorderBox,
            mask_origin: BackgroundBox::BorderBox,
            mask_position: BackgroundPosition::default(),
            mask_size: BackgroundSize::Auto,
            mask_repeat: BackgroundRepeat::Repeat,

            font_variant: FontVariant::Normal,
            font_stretch: FontStretch::Normal,
            font_kerning: FontKerning::Auto,
            font_feature_settings: None,

            hyphens: Hyphens::Manual,
            text_align_last: TextAlignLast::Auto,
            text_justify: TextJustify::Auto,
            text_underline_position: TextUnderlinePosition::Auto,
            text_decoration_thickness: None,
            text_rendering: TextRendering::Auto,

            clip_path: None,
            perspective: None,
            perspective_origin: None,
            backface_visibility: BackfaceVisibility::Visible,
            transform_style: TransformStyle::Flat,

            justify_self: JustifySelf::Auto,
            justify_items: JustifySelf::Stretch,

            custom_properties: FxHashMap::default(),

            container_type: ContainerType::Normal,
            container_name: Vec::new(),

            field_sizing: FieldSizing::Fixed,
        }
    }
}

impl ComputedStyle {
    /// Return the serialized CSS value for a given property name.
    ///
    /// Handles the ~20 most common properties queried via
    /// `getComputedStyle().getPropertyValue()`.
    pub fn get_property_value(&self, property: &str) -> String {
        fn color_to_css(c: Color) -> String {
            if c.a == 255 {
                format!("rgb({}, {}, {})", c.r, c.g, c.b)
            } else {
                let a = f64::from(c.a) / 255.0;
                format!("rgba({}, {}, {}, {a:.2})", c.r, c.g, c.b)
            }
        }
        fn dim_to_css(d: Dimension) -> String {
            match d {
                Dimension::Auto => "auto".into(),
                Dimension::Px(v) => format!("{v}px"),
                Dimension::Percent(v) => format!("{v}%"),
                Dimension::MinContent => "min-content".into(),
                Dimension::MaxContent => "max-content".into(),
                Dimension::FitContent => "fit-content".into(),
            }
        }
        match property {
            "color" => color_to_css(self.color),
            "background-color" => color_to_css(self.background_color),
            "display" => match self.display {
                Display::Block => "block",
                Display::Inline => "inline",
                Display::InlineBlock => "inline-block",
                Display::Flex => "flex",
                Display::InlineFlex => "inline-flex",
                Display::Grid => "grid",
                Display::InlineGrid => "inline-grid",
                Display::ListItem => "list-item",
                Display::Table => "table",
                Display::TableRow => "table-row",
                Display::TableCell => "table-cell",
                Display::None => "none",
            }
            .into(),
            "position" => match self.position {
                Position::Static => "static",
                Position::Relative => "relative",
                Position::Absolute => "absolute",
                Position::Fixed => "fixed",
                Position::Sticky => "sticky",
            }
            .into(),
            "visibility" => match self.visibility {
                Visibility::Visible => "visible",
                Visibility::Hidden => "hidden",
            }
            .into(),
            "font-size" => format!("{}px", self.font_size),
            "font-weight" => format!("{}", self.font_weight.0),
            "line-height" => format!("{}px", self.line_height),
            "width" => dim_to_css(self.width),
            "height" => dim_to_css(self.height),
            "margin-top" => format!("{}px", self.margin_top),
            "margin-right" => format!("{}px", self.margin_right),
            "margin-bottom" => format!("{}px", self.margin_bottom),
            "margin-left" => format!("{}px", self.margin_left),
            "padding-top" => format!("{}px", self.padding_top),
            "padding-right" => format!("{}px", self.padding_right),
            "padding-bottom" => format!("{}px", self.padding_bottom),
            "padding-left" => format!("{}px", self.padding_left),
            "border-top-width" => format!("{}px", self.border_top_width),
            "border-right-width" => format!("{}px", self.border_right_width),
            "border-bottom-width" => format!("{}px", self.border_bottom_width),
            "border-left-width" => format!("{}px", self.border_left_width),
            "opacity" => format!("{}", self.opacity),
            "z-index" => {
                if self.z_index_auto {
                    "auto".into()
                } else {
                    format!("{}", self.z_index)
                }
            },
            "overflow" => match self.overflow {
                Overflow::Visible => "visible",
                Overflow::Hidden => "hidden",
                Overflow::Scroll => "scroll",
                Overflow::Auto => "auto",
            }
            .into(),
            "text-align" => match self.text_align {
                TextAlign::Left => "left",
                TextAlign::Right => "right",
                TextAlign::Center => "center",
                TextAlign::Justify => "justify",
            }
            .into(),
            "border-radius" => {
                if self.border_radius.is_uniform() {
                    format!("{}px", self.border_radius.top_left)
                } else {
                    format!(
                        "{}px {}px {}px {}px",
                        self.border_radius.top_left,
                        self.border_radius.top_right,
                        self.border_radius.bottom_right,
                        self.border_radius.bottom_left,
                    )
                }
            },
            "float" => match self.float {
                Float::None => "none",
                Float::Left => "left",
                Float::Right => "right",
            }
            .into(),
            "max-width" => dim_to_css(self.max_width),
            "max-height" => dim_to_css(self.max_height),
            "min-width" => dim_to_css(self.min_width),
            "min-height" => dim_to_css(self.min_height),
            "top" => dim_to_css(self.top),
            "right" => dim_to_css(self.right),
            "bottom" => dim_to_css(self.bottom),
            "left" => dim_to_css(self.left),
            "cursor" => match self.cursor {
                Cursor::Auto => "auto",
                Cursor::Default => "default",
                Cursor::Pointer => "pointer",
                Cursor::Text => "text",
                Cursor::Move => "move",
                Cursor::NotAllowed => "not-allowed",
                Cursor::Crosshair => "crosshair",
                Cursor::Wait => "wait",
                Cursor::Help => "help",
                Cursor::Grab => "grab",
                Cursor::Grabbing => "grabbing",
                Cursor::None => "none",
                _ => "auto",
            }
            .into(),
            "pointer-events" => match self.pointer_events {
                PointerEvents::Auto => "auto",
                PointerEvents::None => "none",
            }
            .into(),
            "user-select" => match self.user_select {
                UserSelect::Auto => "auto",
                UserSelect::None => "none",
                UserSelect::Text => "text",
                UserSelect::All => "all",
            }
            .into(),
            "aspect-ratio" => match self.aspect_ratio {
                Some(r) => format!("{r}"),
                None => "auto".into(),
            },
            "direction" => match self.direction {
                TextDirection::Ltr | TextDirection::Auto => "ltr",
                TextDirection::Rtl => "rtl",
            }
            .into(),
            _ => String::new(),
        }
    }

    /// Create an initial style that inherits inheritable properties from
    /// the given parent style. Non-inheritable properties keep their
    /// CSS initial values.
    pub fn inherit(parent: &ComputedStyle) -> Self {
        ComputedStyle {
            // Inherited text properties.
            color: parent.color,
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            font_family: parent.font_family,
            text_align: parent.text_align,
            direction: parent.direction,
            text_decoration: parent.text_decoration,
            text_indent: parent.text_indent,
            text_transform: parent.text_transform,
            line_height: parent.line_height,
            line_height_factor: parent.line_height_factor,
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            white_space: parent.white_space,
            text_wrap: parent.text_wrap,
            // Inherited text shadow.
            text_shadow: parent.text_shadow,
            // Inherited visibility.
            visibility: parent.visibility,
            // Inherited list properties.
            list_style_type: parent.list_style_type,
            list_style_position: parent.list_style_position,
            // Inherited table properties.
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            // Inherited interaction properties.
            cursor: parent.cursor,
            pointer_events: parent.pointer_events,
            user_select: parent.user_select,
            // Inherited font extensions.
            font_variant: parent.font_variant,
            font_stretch: parent.font_stretch,
            font_kerning: parent.font_kerning,
            font_feature_settings: parent.font_feature_settings.clone(),
            // Inherited text extensions.
            hyphens: parent.hyphens,
            text_align_last: parent.text_align_last,
            text_justify: parent.text_justify,
            text_underline_position: parent.text_underline_position,
            text_rendering: parent.text_rendering,
            image_rendering: parent.image_rendering,
            // CSS custom properties always inherit.
            custom_properties: parent.custom_properties.clone(),
            // Non-inherited properties keep CSS initial values.
            ..ComputedStyle::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_has_browser_defaults() {
        let s = ComputedStyle::default();
        assert_eq!(s.display, Display::Inline);
        assert_eq!(s.visibility, Visibility::Visible);
        assert_eq!(s.color, Color::BLACK);
        assert!((s.font_size - ROOT_FONT_SIZE).abs() < f32::EPSILON);
        assert_eq!(s.font_weight, FontWeight::NORMAL);
        assert_eq!(s.font_style, FontStyle::Normal);
        assert_eq!(s.font_family, FontFamily::SansSerif);
        assert!((s.line_height - ROOT_FONT_SIZE * 1.5).abs() < 0.01);
        assert!((s.margin_top).abs() < f32::EPSILON);
        assert!((s.padding_top).abs() < f32::EPSILON);
        assert!((s.border_top_width).abs() < f32::EPSILON);
        assert_eq!(s.background_color, Color::rgba(0, 0, 0, 0));
        assert_eq!(s.float, Float::None);
        assert_eq!(s.overflow, Overflow::Visible);
        assert_eq!(s.text_align, TextAlign::Left);
        assert_eq!(s.text_decoration, TextDecoration::NONE);
        assert_eq!(s.white_space, WhiteSpace::Normal);
        assert_eq!(s.list_style_type, ListStyleType::Disc);
        assert_eq!(s.border_collapse, BorderCollapse::Separate);
    }

    #[test]
    fn inherit_copies_inheritable_properties() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        parent.font_size = 20.0;
        parent.font_weight = FontWeight::BOLD;
        parent.text_align = TextAlign::Center;
        parent.visibility = Visibility::Hidden;
        parent.list_style_type = ListStyleType::Square;

        let child = ComputedStyle::inherit(&parent);

        // Inherited.
        assert_eq!(child.color, Color::rgb(255, 0, 0));
        assert!((child.font_size - 20.0).abs() < f32::EPSILON);
        assert_eq!(child.font_weight, FontWeight::BOLD);
        assert_eq!(child.text_align, TextAlign::Center);
        assert_eq!(child.visibility, Visibility::Hidden);
        assert_eq!(child.list_style_type, ListStyleType::Square);

        // Non-inherited: should be initial values, not parent's.
        assert_eq!(child.display, Display::Inline);
        assert!((child.margin_top).abs() < f32::EPSILON);
        assert_eq!(child.background_color, Color::rgba(0, 0, 0, 0));
        assert_eq!(child.float, Float::None);
    }

    mod prop {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// ComputedStyle::inherit preserves inheritable props.
            #[test]
            fn inherit_preserves_font_size(fs in 1.0f32..100.0) {
                let mut parent = ComputedStyle::default();
                parent.font_size = fs;
                let child = ComputedStyle::inherit(&parent);
                prop_assert!(
                    (child.font_size - fs).abs() < 0.001,
                    "inherited font_size: got {}, expected {fs}",
                    child.font_size,
                );
            }

            /// ComputedStyle::inherit resets non-inheritable props.
            #[test]
            fn inherit_resets_margin(
                mt in 1.0f32..100.0,
                mr in 1.0f32..100.0,
            ) {
                let mut parent = ComputedStyle::default();
                parent.margin_top = mt;
                parent.margin_right = mr;
                let child = ComputedStyle::inherit(&parent);
                prop_assert!(
                    child.margin_top.abs() < 0.001,
                    "margin_top should be reset, got {}",
                    child.margin_top,
                );
                prop_assert!(
                    child.margin_right.abs() < 0.001,
                    "margin_right should be reset, got {}",
                    child.margin_right,
                );
            }
        }
    }
}
