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
                        for inner_simple in &inner.parts {
                            match inner_simple {
                                SimpleSelector::Id(_) => {
                                    ids = ids.saturating_add(1);
                                },
                                SimpleSelector::Class(_)
                                | SimpleSelector::PseudoClass(_)
                                | SimpleSelector::PseudoClassFn(_, _)
                                | SimpleSelector::Attribute { .. } => {
                                    classes = classes.saturating_add(1);
                                },
                                SimpleSelector::Type(_) => {
                                    types = types.saturating_add(1);
                                },
                                _ => {},
                            }
                        }
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

// -------------------------------------------------------------------
// Declaration / value types
// -------------------------------------------------------------------

/// A single CSS property declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: CssValue,
    pub important: bool,
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

/// A complete parsed stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}
