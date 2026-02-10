// HTML named character reference lookup table.
//
// Covers the ~100 most commonly used entities. Case-sensitive, matching the
// HTML specification. Names are given *without* the leading `&` and trailing
// `;` (e.g. pass `"amp"` not `"&amp;"`).

/// Look up a named HTML character reference (without the leading `&`
/// and trailing `;`).
/// Returns the character(s) if found, or `None` for unknown references.
pub fn lookup_entity(name: &str) -> Option<&'static str> {
    let s: &'static str = match name {
        // ---- Essential / XML predefined -----------------------------------
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",

        // ---- Whitespace ---------------------------------------------------
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "thinsp" => "\u{2009}",

        // ---- Typography ---------------------------------------------------
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "hellip" => "\u{2026}",
        "bull" => "\u{2022}",
        "middot" => "\u{00B7}",
        "laquo" => "\u{00AB}",
        "raquo" => "\u{00BB}",

        // ---- Symbols ------------------------------------------------------
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "times" => "\u{00D7}",
        "divide" => "\u{00F7}",
        "plusmn" => "\u{00B1}",
        "deg" => "\u{00B0}",
        "micro" => "\u{00B5}",
        "para" => "\u{00B6}",
        "sect" => "\u{00A7}",
        "cent" => "\u{00A2}",
        "pound" => "\u{00A3}",
        "yen" => "\u{00A5}",
        "euro" => "\u{20AC}",
        "curren" => "\u{00A4}",

        // ---- Arrows -------------------------------------------------------
        "larr" => "\u{2190}",
        "uarr" => "\u{2191}",
        "rarr" => "\u{2192}",
        "darr" => "\u{2193}",
        "harr" => "\u{2194}",

        // ---- Math ---------------------------------------------------------
        "frac14" => "\u{00BC}",
        "frac12" => "\u{00BD}",
        "frac34" => "\u{00BE}",
        "ne" => "\u{2260}",
        "le" => "\u{2264}",
        "ge" => "\u{2265}",
        "infin" => "\u{221E}",
        "sum" => "\u{2211}",
        "prod" => "\u{220F}",
        "radic" => "\u{221A}",
        "minus" => "\u{2212}",
        "lowast" => "\u{2217}",
        "sim" => "\u{223C}",
        "asymp" => "\u{2248}",
        "equiv" => "\u{2261}",
        "fnof" => "\u{0192}",

        // ---- Accented Latin (uppercase) -----------------------------------
        "Agrave" => "\u{00C0}",
        "Aacute" => "\u{00C1}",
        "Acirc" => "\u{00C2}",
        "Atilde" => "\u{00C3}",
        "Auml" => "\u{00C4}",
        "Aring" => "\u{00C5}",
        "AElig" => "\u{00C6}",
        "Ccedil" => "\u{00C7}",
        "Egrave" => "\u{00C8}",
        "Eacute" => "\u{00C9}",
        "Ecirc" => "\u{00CA}",
        "Euml" => "\u{00CB}",
        "Igrave" => "\u{00CC}",
        "Iacute" => "\u{00CD}",
        "Icirc" => "\u{00CE}",
        "Iuml" => "\u{00CF}",
        "ETH" => "\u{00D0}",
        "Ntilde" => "\u{00D1}",
        "Ograve" => "\u{00D2}",
        "Oacute" => "\u{00D3}",
        "Ocirc" => "\u{00D4}",
        "Otilde" => "\u{00D5}",
        "Ouml" => "\u{00D6}",
        "Oslash" => "\u{00D8}",
        "Ugrave" => "\u{00D9}",
        "Uacute" => "\u{00DA}",
        "Ucirc" => "\u{00DB}",
        "Uuml" => "\u{00DC}",
        "Yacute" => "\u{00DD}",
        "THORN" => "\u{00DE}",

        // ---- Accented Latin (lowercase) -----------------------------------
        "szlig" => "\u{00DF}",
        "agrave" => "\u{00E0}",
        "aacute" => "\u{00E1}",
        "acirc" => "\u{00E2}",
        "atilde" => "\u{00E3}",
        "auml" => "\u{00E4}",
        "aring" => "\u{00E5}",
        "aelig" => "\u{00E6}",
        "ccedil" => "\u{00E7}",
        "egrave" => "\u{00E8}",
        "eacute" => "\u{00E9}",
        "ecirc" => "\u{00EA}",
        "euml" => "\u{00EB}",
        "igrave" => "\u{00EC}",
        "iacute" => "\u{00ED}",
        "icirc" => "\u{00EE}",
        "iuml" => "\u{00EF}",
        "eth" => "\u{00F0}",
        "ntilde" => "\u{00F1}",
        "ograve" => "\u{00F2}",
        "oacute" => "\u{00F3}",
        "ocirc" => "\u{00F4}",
        "otilde" => "\u{00F5}",
        "ouml" => "\u{00F6}",
        "oslash" => "\u{00F8}",
        "ugrave" => "\u{00F9}",
        "uacute" => "\u{00FA}",
        "ucirc" => "\u{00FB}",
        "uuml" => "\u{00FC}",
        "yacute" => "\u{00FD}",
        "thorn" => "\u{00FE}",
        "yuml" => "\u{00FF}",

        // ---- Greek (uppercase) --------------------------------------------
        "Alpha" => "\u{0391}",
        "Beta" => "\u{0392}",
        "Gamma" => "\u{0393}",
        "Delta" => "\u{0394}",
        "Epsilon" => "\u{0395}",
        "Zeta" => "\u{0396}",
        "Eta" => "\u{0397}",
        "Theta" => "\u{0398}",
        "Iota" => "\u{0399}",
        "Kappa" => "\u{039A}",
        "Lambda" => "\u{039B}",
        "Mu" => "\u{039C}",
        "Nu" => "\u{039D}",
        "Xi" => "\u{039E}",
        "Omicron" => "\u{039F}",
        "Pi" => "\u{03A0}",
        "Rho" => "\u{03A1}",
        "Sigma" => "\u{03A3}",
        "Tau" => "\u{03A4}",
        "Upsilon" => "\u{03A5}",
        "Phi" => "\u{03A6}",
        "Chi" => "\u{03A7}",
        "Psi" => "\u{03A8}",
        "Omega" => "\u{03A9}",

        // ---- Greek (lowercase) --------------------------------------------
        "alpha" => "\u{03B1}",
        "beta" => "\u{03B2}",
        "gamma" => "\u{03B3}",
        "delta" => "\u{03B4}",
        "epsilon" => "\u{03B5}",
        "zeta" => "\u{03B6}",
        "eta" => "\u{03B7}",
        "theta" => "\u{03B8}",
        "iota" => "\u{03B9}",
        "kappa" => "\u{03BA}",
        "lambda" => "\u{03BB}",
        "mu" => "\u{03BC}",
        "nu" => "\u{03BD}",
        "xi" => "\u{03BE}",
        "omicron" => "\u{03BF}",
        "pi" => "\u{03C0}",
        "rho" => "\u{03C1}",
        "sigmaf" => "\u{03C2}",
        "sigma" => "\u{03C3}",
        "tau" => "\u{03C4}",
        "upsilon" => "\u{03C5}",
        "phi" => "\u{03C6}",
        "chi" => "\u{03C7}",
        "psi" => "\u{03C8}",
        "omega" => "\u{03C9}",

        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn essential_entities() {
        assert_eq!(lookup_entity("amp"), Some("&"));
        assert_eq!(lookup_entity("lt"), Some("<"));
        assert_eq!(lookup_entity("gt"), Some(">"));
        assert_eq!(lookup_entity("quot"), Some("\""));
        assert_eq!(lookup_entity("apos"), Some("'"));
        assert_eq!(lookup_entity("nbsp"), Some("\u{00A0}"));
    }

    #[test]
    fn typography() {
        assert_eq!(lookup_entity("mdash"), Some("\u{2014}"));
        assert_eq!(lookup_entity("ndash"), Some("\u{2013}"));
        assert_eq!(lookup_entity("hellip"), Some("\u{2026}"));
        assert_eq!(lookup_entity("ldquo"), Some("\u{201C}"));
        assert_eq!(lookup_entity("rdquo"), Some("\u{201D}"));
    }

    #[test]
    fn symbols_and_currency() {
        assert_eq!(lookup_entity("copy"), Some("\u{00A9}"));
        assert_eq!(lookup_entity("reg"), Some("\u{00AE}"));
        assert_eq!(lookup_entity("trade"), Some("\u{2122}"));
        assert_eq!(lookup_entity("euro"), Some("\u{20AC}"));
        assert_eq!(lookup_entity("pound"), Some("\u{00A3}"));
        assert_eq!(lookup_entity("deg"), Some("\u{00B0}"));
    }

    #[test]
    fn math_entities() {
        assert_eq!(lookup_entity("frac12"), Some("\u{00BD}"));
        assert_eq!(lookup_entity("ne"), Some("\u{2260}"));
        assert_eq!(lookup_entity("infin"), Some("\u{221E}"));
        assert_eq!(lookup_entity("sum"), Some("\u{2211}"));
    }

    #[test]
    fn accented_latin() {
        assert_eq!(lookup_entity("Agrave"), Some("\u{00C0}"));
        assert_eq!(lookup_entity("eacute"), Some("\u{00E9}"));
        assert_eq!(lookup_entity("Ntilde"), Some("\u{00D1}"));
        assert_eq!(lookup_entity("ntilde"), Some("\u{00F1}"));
        assert_eq!(lookup_entity("szlig"), Some("\u{00DF}"));
        assert_eq!(lookup_entity("Uuml"), Some("\u{00DC}"));
        assert_eq!(lookup_entity("uuml"), Some("\u{00FC}"));
    }

    #[test]
    fn greek_letters() {
        assert_eq!(lookup_entity("Alpha"), Some("\u{0391}"));
        assert_eq!(lookup_entity("alpha"), Some("\u{03B1}"));
        assert_eq!(lookup_entity("Omega"), Some("\u{03A9}"));
        assert_eq!(lookup_entity("omega"), Some("\u{03C9}"));
        assert_eq!(lookup_entity("pi"), Some("\u{03C0}"));
        assert_eq!(lookup_entity("sigma"), Some("\u{03C3}"));
        assert_eq!(lookup_entity("sigmaf"), Some("\u{03C2}"));
    }

    #[test]
    fn arrows() {
        assert_eq!(lookup_entity("larr"), Some("\u{2190}"));
        assert_eq!(lookup_entity("rarr"), Some("\u{2192}"));
        assert_eq!(lookup_entity("uarr"), Some("\u{2191}"));
        assert_eq!(lookup_entity("darr"), Some("\u{2193}"));
    }

    #[test]
    fn case_sensitivity() {
        // HTML entities are case-sensitive.
        assert_eq!(lookup_entity("Agrave"), Some("\u{00C0}"));
        assert_eq!(lookup_entity("agrave"), Some("\u{00E0}"));
        assert_ne!(lookup_entity("Agrave"), lookup_entity("agrave"));
        // Non-existent casing returns None.
        assert_eq!(lookup_entity("AMP"), None);
        assert_eq!(lookup_entity("LT"), None);
    }

    #[test]
    fn unknown_entity_returns_none() {
        assert_eq!(lookup_entity(""), None);
        assert_eq!(lookup_entity("notareal"), None);
        assert_eq!(lookup_entity("foobar"), None);
    }
}
