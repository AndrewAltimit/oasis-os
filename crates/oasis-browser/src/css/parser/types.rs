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
    /// Negation: `:not(selector-list)`. Matches an element that does
    /// not match any compound in the list (Selectors Level 4 form).
    Not(Vec<CompoundSelector>),
    /// `:is(selector-list)` -- matches if any inner selector matches.
    Is(Vec<CompoundSelector>),
    /// `:where(selector-list)` -- like `:is()` but zero specificity.
    Where(Vec<CompoundSelector>),
    /// `:has(relative-selector-list)` -- the relational pseudo-class.
    /// Each entry is a leading combinator (default [`Combinator::Descendant`])
    /// and a full selector evaluated relative to the subject element.
    Has(Vec<(Combinator, Selector)>),
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
                    SimpleSelector::Not(inner_list) => {
                        // :not() itself doesn't count, but the most
                        // specific compound in its argument list does.
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
                    SimpleSelector::Has(inner_list) => {
                        // :has() takes the max specificity across the
                        // relative selectors it contains, the same rule
                        // as :is().
                        let max_spec = inner_list
                            .iter()
                            .map(|(_, sel)| sel.specificity())
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
            SimpleSelector::Not(inner_list) => {
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
            SimpleSelector::Has(inner_list) => {
                let max_spec = inner_list
                    .iter()
                    .map(|(_, sel)| sel.specificity())
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
    /// A deferred `light-dark(light, dark)` color — resolved at
    /// computed-value time based on the element's `color-scheme`.
    LightDark(CssColor, CssColor),
}

/// Supported CSS length units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthUnit {
    Px,
    Em,
    Rem,
    Pt,
    /// `ex` — nominal x-height. Resolved as 0.5em, matching what every
    /// real browser does when fontdue hasn't been queried yet. Old.reddit
    /// leans on this unit for its `.midcol` (`width: 4.1ex`) and `.rank`
    /// (`width: 2.2ex`) vote column, so treating it as 0 collapses the
    /// whole vote gutter.
    Ex,
    /// `ch` — nominal advance width of '0'. Resolved as 0.5em (same
    /// heuristic as `ex`; close enough for proportional fonts and matches
    /// how author CSS expects it to behave for sizing terminal columns).
    Ch,
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
    /// Cascade-layer membership: index into the parent stylesheet's
    /// [`Stylesheet::layers`] list, or `None` if the rule is unlayered.
    /// Unlayered author rules win over layered author rules (for normal
    /// declarations); `!important` reverses the order within layers.
    pub layer: Option<u16>,
    /// `@container` condition the rule was nested inside, if any. The
    /// rule only contributes its declarations when this condition
    /// evaluates true against the nearest matching container ancestor
    /// at cascade time. `None` for unconditional rules.
    pub container: Option<ContainerCondition>,
    /// `@scope (root) [to (limit)]?` condition the rule was nested
    /// inside, if any. The rule only matches elements that are
    /// descendants of (or equal to) some `root` ancestor and not below
    /// any matching `limit` boundary. `None` for unconditional rules.
    pub scope: Option<ScopeCondition>,
}

/// Parsed `@scope` condition. Both root and limit are stored as the
/// raw selector text — we re-parse on demand at cascade time via
/// `parse_selector_string` so we can reuse the full selector engine
/// without duplicating it here.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeCondition {
    /// The `(root-selector)` text. `None` means "implicitly scoped to
    /// the stylesheet's owner element" — for a `<style>` block we treat
    /// that as "matches anywhere in the document" (no root constraint).
    pub root: Option<String>,
    /// The `to (limit-selector)` text, if any. Elements that match the
    /// limit selector — and all of their descendants — fall outside the
    /// scope and don't get the rule applied.
    pub limit: Option<String>,
}

/// Parsed `@container` condition: optional container name plus a
/// conjunction of size feature predicates (`min-width`, `max-width`,
/// `min-height`, `max-height`, `width`, `height`, plus the
/// `inline-size` / `block-size` aliases).
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerCondition {
    /// Optional container name. `None` matches any unnamed or named
    /// container ancestor; `Some(name)` matches only ancestors whose
    /// `container-name` includes this identifier.
    pub name: Option<String>,
    /// Feature predicates joined with `and`. All must hold against the
    /// nearest matching container's size for the rule to apply.
    pub features: Vec<ContainerFeature>,
}

/// A single `@container` feature predicate.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerFeature {
    MinWidth(f32),
    MaxWidth(f32),
    Width(f32),
    MinHeight(f32),
    MaxHeight(f32),
    Height(f32),
    /// `style(property: value)` — matches when the container's computed
    /// style for the given property equals the value string.
    Style(String, String),
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

/// A parsed `@counter-style name { ... }` rule. The descriptors are
/// stored as raw strings — we keep this lossless for round-tripping
/// but don't yet wire it into list-item rendering, so most fields are
/// just remembered for `getCounterStyle()`-style introspection.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterStyleRule {
    /// The author-supplied counter style name (e.g. `thumbs`).
    pub name: String,
    /// `system: cyclic | numeric | alphabetic | symbolic | additive |
    /// fixed | extends ...`. Stored as a lowercased keyword.
    pub system: Option<String>,
    /// `symbols: ...` — the visible glyphs / strings, in order.
    pub symbols: Vec<String>,
    /// `additive-symbols: ...` for the `additive` system. Pairs of
    /// `(weight, symbol)`.
    pub additive_symbols: Vec<(i32, String)>,
    /// `range: ...` raw text (e.g. `1 5`, `auto`, `infinite infinite`).
    pub range: Option<String>,
    /// `prefix: "x"` — text inserted before the marker.
    pub prefix: Option<String>,
    /// `suffix: "x"` — text inserted after the marker.
    pub suffix: Option<String>,
    /// `pad: <integer> <symbol>`, stored as raw text.
    pub pad: Option<String>,
    /// `negative: <prefix> [<suffix>]`, raw text.
    pub negative: Option<String>,
    /// `fallback: <name>` — counter style to fall back to.
    pub fallback: Option<String>,
    /// `speak-as: ...` — accessibility fallback. Raw text.
    pub speak_as: Option<String>,
}

/// A parsed `@property --foo { ... }` registration. Lets authors
/// declare a typed custom property with a default initial value and
/// inheritance flag. The cascade falls back to `initial_value` when
/// `var(--foo)` is referenced and not overridden by any declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRule {
    /// Property name including the leading `--` (e.g. `--brand-color`).
    pub name: String,
    /// `syntax: "<color>"` etc. — kept as the raw author string. We
    /// don't enforce the syntax grammar yet, so any value is accepted.
    pub syntax: Option<String>,
    /// `inherits: true | false`. Defaults to `false` per spec when
    /// the descriptor is missing (which is technically a parse error
    /// in real browsers, but we accept it).
    pub inherits: bool,
    /// `initial-value: ...` — raw value string used as the fallback
    /// for unresolved `var()` lookups. `None` means no initial value.
    pub initial_value: Option<String>,
}

/// A single `src:` entry in an `@font-face` rule.
///
/// Represents one font source — either a URL (`url("...")`) with
/// optional `format()` hints, or a local font name (`local("...")`).
#[derive(Debug, Clone, PartialEq)]
pub enum FontFaceSrc {
    /// `url("path/to/font.woff2") format("woff2")`
    Url {
        url: String,
        /// Optional format hints (e.g. `"woff2"`, `"truetype"`).
        format: Vec<String>,
    },
    /// `local("Helvetica Neue")`
    Local(String),
}

/// CSS `font-display` descriptor for `@font-face`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontDisplay {
    /// Invisible fallback for a short period, then swap.
    #[default]
    Auto,
    /// Invisible for a long time, then swap.
    Block,
    /// Very short invisible period, then fallback forever.
    Swap,
    /// Very short invisible period, then fallback, late swap.
    Fallback,
    /// Use fallback immediately; only swap if font loads fast.
    Optional,
}

/// A single Unicode range, e.g. `U+0020-007F` or `U+4?`.
///
/// Stored as an inclusive `[start, end]` codepoint range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicodeRange {
    pub start: u32,
    pub end: u32,
}

/// A parsed `@font-face { ... }` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    /// `font-family` descriptor — the name authors use to reference
    /// this font in `font-family:` declarations (required).
    pub family: String,
    /// `src:` descriptor — ordered list of font sources.
    pub src: Vec<FontFaceSrc>,
    /// `font-weight` descriptor. Stored as numeric values (100–900).
    /// A single value `(w, w)` or a range `(w1, w2)`.
    pub weight: (u16, u16),
    /// `font-style` descriptor.
    pub style: FontFaceStyle,
    /// `font-display` descriptor.
    pub display: FontDisplay,
    /// `unicode-range` descriptor — restricts which codepoints this
    /// font covers. Empty means the full Unicode range.
    pub unicode_range: Vec<UnicodeRange>,
}

/// `font-style` values for `@font-face`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFaceStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

/// A complete parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub keyframes: Vec<KeyframesRule>,
    /// Cascade layers declared in this stylesheet, in declaration
    /// order. Each `Rule`'s `layer` field is an index into this list.
    /// Anonymous layers (from `@layer { ... }`) get a synthetic name
    /// of the form `"__anon_{n}"` so they stay distinct from any
    /// named layer. Empty for stylesheets without any `@layer` rules.
    pub layers: Vec<String>,
    /// `@counter-style` registrations declared in this stylesheet.
    pub counter_styles: Vec<CounterStyleRule>,
    /// `@property` registrations declared in this stylesheet. Each
    /// supplies an `initial-value` fallback that the cascade seeds
    /// into the element's custom-properties map before resolving any
    /// `var()` references that target the registered property.
    pub properties: Vec<PropertyRule>,
    /// `@font-face` rules declared in this stylesheet. Each registers
    /// a web font with family name, source URL(s), weight/style ranges,
    /// and optional unicode-range/font-display descriptors.
    pub font_faces: Vec<FontFaceRule>,
}
