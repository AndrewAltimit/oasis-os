//! Public types for the CSS parser.
//!
//! Selector types, declaration/value types, specificity, rules, and
//! the [`Stylesheet`] container.

// -------------------------------------------------------------------
// Selector types
// -------------------------------------------------------------------

/// Attribute selector match operator.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrOp {
    /// `[attr]` -- attribute exists.
    Exists,
    /// `[attr=val]` -- exact match.
    Equals,
    /// `[attr~=val]` -- space-separated word match.
    Includes,
    /// `[attr|=val]` -- exact or prefix with hyphen.
    DashMatch,
    /// `[attr^=val]` -- starts with.
    Prefix,
    /// `[attr$=val]` -- ends with.
    Suffix,
    /// `[attr*=val]` -- substring match.
    Substring,
}

/// A single, atomic selector component.
#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    /// Type selector: `div`, `p`, `h1`.
    Type(String),
    /// Class selector: `.classname`.
    Class(String),
    /// ID selector: `#idname`.
    Id(String),
    /// Universal selector: `*`.
    Universal,
    /// Pseudo-class: `:hover`, `:first-child`.
    PseudoClass(String),
    /// Functional pseudo-class with argument: `:nth-child(2n+1)`.
    PseudoClassFn(String, String),
    /// Pseudo-element: `::before`, `::after`.
    PseudoElement(String),
    /// Negation: `:not(selector)`.
    Not(Box<CompoundSelector>),
    /// `:is(selector-list)` -- matches if any inner selector matches.
    Is(Vec<CompoundSelector>),
    /// `:where(selector-list)` -- like `:is()` but zero specificity.
    Where(Vec<CompoundSelector>),
    /// Nesting selector `&` — a parser-time marker that refers to the
    /// enclosing rule's selector. Desugared into concrete selectors at
    /// parse time; should not appear in the final AST.
    Nest,
    /// Attribute selector: `[attr]`, `[attr=val]`, etc.
    Attribute {
        name: String,
        op: AttrOp,
        value: Option<String>,
    },
}

/// Combinator linking two compound selectors.
#[derive(Debug, Clone, PartialEq)]
pub enum Combinator {
    /// Descendant: `div p` (whitespace).
    Descendant,
    /// Child: `div > p`.
    Child,
    /// Adjacent sibling: `h1 + p`.
    AdjacentSibling,
    /// General sibling: `h1 ~ p`.
    GeneralSibling,
}

/// A compound selector is a sequence of simple selectors applied to the
/// same element (e.g. `div.class#id`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    /// Parts that must all match the same element.
    pub parts: Vec<SimpleSelector>,
}

/// A full selector is a chain of compound selectors separated by
/// combinators.  Each entry stores the compound selector and the
/// combinator that *preceded* it (`None` for the first in the chain).
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<(CompoundSelector, Option<Combinator>)>,
}

/// Comma-separated list of selectors.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList {
    pub selectors: Vec<Selector>,
}

// -------------------------------------------------------------------
// Specificity
// -------------------------------------------------------------------

/// CSS specificity in the standard (inline, id, class, type) tuple form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity {
    /// 1 if the style originates from an inline `style` attribute.
    pub inline: u8,
    /// Count of ID selectors.
    pub ids: u8,
    /// Count of class, pseudo-class, and attribute selectors.
    pub classes: u8,
    /// Count of type selectors and pseudo-elements.
    pub types: u8,
}

impl Selector {
    /// Compute the specificity of this selector.  Inline is always 0 here;
    /// the caller bumps it for inline styles.
    pub fn specificity(&self) -> Specificity {
        let mut ids: u8 = 0;
        let mut classes: u8 = 0;
        let mut types: u8 = 0;
        for (compound, _) in &self.parts {
            for simple in &compound.parts {
                match simple {
                    SimpleSelector::Id(_) => {
                        ids = ids.saturating_add(1);
                    },
                    SimpleSelector::Class(_)
                    | SimpleSelector::PseudoClass(_)
                    | SimpleSelector::PseudoClassFn(_, _)
                    | SimpleSelector::Attribute { .. } => {
                        classes = classes.saturating_add(1);
                    },
                    SimpleSelector::Not(inner) => {
                        // :not() itself doesn't count, but its argument does.
                        let inner_spec = compound_specificity(inner);
                        ids = ids.saturating_add(inner_spec.ids);
                        classes = classes.saturating_add(inner_spec.classes);
                        types = types.saturating_add(inner_spec.types);
                    },
                    SimpleSelector::Is(inner_list) => {
                        // :is() takes the max specificity of its arguments.
                        let max_spec = inner_list.iter().map(compound_specificity).max().unwrap_or(
                            Specificity {
                                inline: 0,
                                ids: 0,
                                classes: 0,
                                types: 0,
                            },
                        );
                        ids = ids.saturating_add(max_spec.ids);
                        classes = classes.saturating_add(max_spec.classes);
                        types = types.saturating_add(max_spec.types);
                    },
                    SimpleSelector::Where(_) => {
                        // :where() contributes zero specificity.
                    },
                    SimpleSelector::Nest => {
                        // Nest is desugared at parse time; any residual
                        // marker contributes zero.
                    },
                    SimpleSelector::Type(_) | SimpleSelector::PseudoElement(_) => {
                        types = types.saturating_add(1);
                    },
                    SimpleSelector::Universal => {},
                }
            }
        }
        Specificity {
            inline: 0,
            ids,
            classes,
            types,
        }
    }
}

/// Compute the specificity contribution of a single compound selector.
///
/// Used by `:not()` and `:is()` to determine the specificity of their
/// inner selector arguments.
fn compound_specificity(compound: &CompoundSelector) -> Specificity {
    let mut ids: u8 = 0;
    let mut classes: u8 = 0;
    let mut types: u8 = 0;
    for simple in &compound.parts {
        match simple {
            SimpleSelector::Id(_) => {
                ids = ids.saturating_add(1);
            },
            SimpleSelector::Class(_)
            | SimpleSelector::PseudoClass(_)
            | SimpleSelector::PseudoClassFn(_, _)
            | SimpleSelector::Attribute { .. } => {
                classes = classes.saturating_add(1);
            },
            SimpleSelector::Not(inner) => {
                let inner_spec = compound_specificity(inner);
                ids = ids.saturating_add(inner_spec.ids);
                classes = classes.saturating_add(inner_spec.classes);
                types = types.saturating_add(inner_spec.types);
            },
            SimpleSelector::Is(inner_list) => {
                let max_spec =
                    inner_list
                        .iter()
                        .map(compound_specificity)
                        .max()
                        .unwrap_or(Specificity {
                            inline: 0,
                            ids: 0,
                            classes: 0,
                            types: 0,
                        });
                ids = ids.saturating_add(max_spec.ids);
                classes = classes.saturating_add(max_spec.classes);
                types = types.saturating_add(max_spec.types);
            },
            SimpleSelector::Where(_) => {},
            SimpleSelector::Nest => {},
            SimpleSelector::Type(_) | SimpleSelector::PseudoElement(_) => {
                types = types.saturating_add(1);
            },
            SimpleSelector::Universal => {},
        }
    }
    Specificity {
        inline: 0,
        ids,
        classes,
        types,
    }
}

// -------------------------------------------------------------------
// Declaration / value types
// -------------------------------------------------------------------

/// A single CSS property declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
    /// Pre-computed property identifier for fast integer dispatch
    /// in `apply_declaration`. Set during parsing.
    pub property_id: PropertyId,
}

/// Interned property identifier for fast integer dispatch.
///
/// Avoids repeated string comparisons in `apply_declaration`. The most
/// commonly used CSS properties are assigned dedicated variants; anything
/// else falls through to `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PropertyId {
    Display,
    Visibility,
    Margin,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    Padding,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    BorderWidth,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderColor,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderStyle,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    Width,
    Height,
    MaxWidth,
    MinWidth,
    MaxHeight,
    MinHeight,
    Color,
    BackgroundColor,
    FontSize,
    FontWeight,
    FontStyle,
    FontFamily,
    TextAlign,
    TextDecoration,
    TextIndent,
    TextTransform,
    LineHeight,
    LetterSpacing,
    WordSpacing,
    WhiteSpace,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    Float,
    Clear,
    Overflow,
    OverflowX,
    OverflowY,
    ZIndex,
    Opacity,
    BorderRadius,
    BoxSizing,
    FlexDirection,
    FlexWrap,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Flex,
    Gap,
    RowGap,
    ColumnGap,
    Order,
    /// A property not in the fast-path list.
    Other,
}

impl PropertyId {
    /// Map a property name string to a `PropertyId`.
    #[inline]
    pub fn from_name(name: &str) -> Self {
        // Use length + first byte as a fast pre-filter to avoid
        // comparing the full string in the common miss case.
        match name {
            "display" => Self::Display,
            "visibility" => Self::Visibility,
            "margin" => Self::Margin,
            "margin-top" => Self::MarginTop,
            "margin-right" => Self::MarginRight,
            "margin-bottom" => Self::MarginBottom,
            "margin-left" => Self::MarginLeft,
            "padding" => Self::Padding,
            "padding-top" => Self::PaddingTop,
            "padding-right" => Self::PaddingRight,
            "padding-bottom" => Self::PaddingBottom,
            "padding-left" => Self::PaddingLeft,
            "border-width" => Self::BorderWidth,
            "border-top-width" => Self::BorderTopWidth,
            "border-right-width" => Self::BorderRightWidth,
            "border-bottom-width" => Self::BorderBottomWidth,
            "border-left-width" => Self::BorderLeftWidth,
            "border-color" => Self::BorderColor,
            "border-top-color" => Self::BorderTopColor,
            "border-right-color" => Self::BorderRightColor,
            "border-bottom-color" => Self::BorderBottomColor,
            "border-left-color" => Self::BorderLeftColor,
            "border-style" => Self::BorderStyle,
            "border-top-style" => Self::BorderTopStyle,
            "border-right-style" => Self::BorderRightStyle,
            "border-bottom-style" => Self::BorderBottomStyle,
            "border-left-style" => Self::BorderLeftStyle,
            "width" => Self::Width,
            "height" => Self::Height,
            "max-width" => Self::MaxWidth,
            "min-width" => Self::MinWidth,
            "max-height" => Self::MaxHeight,
            "min-height" => Self::MinHeight,
            "color" => Self::Color,
            "background-color" => Self::BackgroundColor,
            "font-size" => Self::FontSize,
            "font-weight" => Self::FontWeight,
            "font-style" => Self::FontStyle,
            "font-family" => Self::FontFamily,
            "text-align" => Self::TextAlign,
            "text-decoration" => Self::TextDecoration,
            "text-indent" => Self::TextIndent,
            "text-transform" => Self::TextTransform,
            "line-height" => Self::LineHeight,
            "letter-spacing" => Self::LetterSpacing,
            "word-spacing" => Self::WordSpacing,
            "white-space" => Self::WhiteSpace,
            "position" => Self::Position,
            "top" => Self::Top,
            "right" => Self::Right,
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "float" => Self::Float,
            "clear" => Self::Clear,
            "overflow" => Self::Overflow,
            "overflow-x" => Self::OverflowX,
            "overflow-y" => Self::OverflowY,
            "z-index" => Self::ZIndex,
            "opacity" => Self::Opacity,
            "border-radius" => Self::BorderRadius,
            "box-sizing" => Self::BoxSizing,
            "flex-direction" => Self::FlexDirection,
            "flex-wrap" => Self::FlexWrap,
            "justify-content" => Self::JustifyContent,
            "align-items" => Self::AlignItems,
            "align-self" => Self::AlignSelf,
            "align-content" => Self::AlignContent,
            "flex-grow" => Self::FlexGrow,
            "flex-shrink" => Self::FlexShrink,
            "flex-basis" => Self::FlexBasis,
            "flex" => Self::Flex,
            "gap" => Self::Gap,
            "row-gap" => Self::RowGap,
            "column-gap" => Self::ColumnGap,
            "order" => Self::Order,
            _ => Self::Other,
        }
    }
}

impl Declaration {
    /// Create a new `Declaration` with the `property_id` computed automatically.
    #[inline]
    pub fn new(property: String, value: CssValue, important: bool) -> Self {
        let property_id = PropertyId::from_name(&property);
        Self {
            property,
            value,
            important,
            property_id,
        }
    }
}

/// A parsed CSS value.
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    /// An unresolved keyword (e.g. `auto`, `inherit`, `solid`).
    Keyword(String),
    /// A length with unit.
    Length(f32, LengthUnit),
    /// A percentage value.
    Percentage(f32),
    /// A resolved colour.
    Color(CssColor),
    /// A bare number.
    Number(f32),
    /// Multiple values (shorthand expansions, font stacks, etc.).
    Multiple(Vec<CssValue>),
    /// A quoted string value.
    String(String),
    /// A `var(--name)` or `var(--name, fallback)` reference.
    Var(String, Option<String>),
    /// A `url(...)` value.
    Url(String),
    /// A parsed `linear-gradient(...)` value.
    Gradient(crate::css::values::LinearGradient),
    /// A parsed `radial-gradient(...)` value.
    RadialGradient(crate::css::values::RadialGradient),
    /// A `calc(...)` expression (raw expression string).
    Calc(String),
}

/// Supported CSS length units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Pt,
}

/// An RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl CssColor {
    pub(crate) const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

// -------------------------------------------------------------------
// Rule / Stylesheet
// -------------------------------------------------------------------

/// A style rule (selector list + declarations).
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: SelectorList,
    pub declarations: Vec<Declaration>,
}

/// A single keyframe stop (percentage + declarations).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeStop {
    /// Percentage 0.0 ..= 100.0.
    pub percentage: f32,
    pub declarations: Vec<Declaration>,
}

/// A parsed `@keyframes` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub stops: Vec<KeyframeStop>,
}

/// A complete parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub keyframes: Vec<KeyframesRule>,
}
