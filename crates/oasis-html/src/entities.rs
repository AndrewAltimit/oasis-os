// HTML named character reference lookup table.
//
// Covers every HTML 4 entity plus common HTML5 names (~300). Case-sensitive, matching the
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

        // ---- Whitespace / formatting characters ----------------------------
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "thinsp" => "\u{2009}",
        "zwnj" => "\u{200C}",
        "zwj" => "\u{200D}",
        "lrm" => "\u{200E}",
        "rlm" => "\u{200F}",

        // ---- Punctuation marks -------------------------------------------
        "iexcl" => "\u{00A1}",
        "iquest" => "\u{00BF}",
        "dagger" => "\u{2020}",
        "Dagger" => "\u{2021}",
        "permil" => "\u{2030}",
        "prime" => "\u{2032}",
        "Prime" => "\u{2033}",

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
        "thetasym" => "\u{03D1}",
        "upsih" => "\u{03D2}",
        "piv" => "\u{03D6}",

        // ---- The rest of HTML 4 (Latin-1, symbols, specials) -------------
        "AMP" => "&",
        "LT" => "<",
        "GT" => ">",
        "QUOT" => "\"",
        "COPY" => "\u{00A9}",
        "REG" => "\u{00AE}",
        "acute" => "\u{00B4}",
        "brvbar" => "\u{00A6}",
        "cedil" => "\u{00B8}",
        "macr" => "\u{00AF}",
        "not" => "\u{00AC}",
        "ordf" => "\u{00AA}",
        "ordm" => "\u{00BA}",
        "shy" => "\u{00AD}",
        "sup1" => "\u{00B9}",
        "sup2" => "\u{00B2}",
        "sup3" => "\u{00B3}",
        "uml" => "\u{00A8}",
        "Yuml" => "\u{0178}",
        "circ" => "\u{02C6}",
        "tilde" => "\u{02DC}",
        "OElig" => "\u{0152}",
        "oelig" => "\u{0153}",
        "Scaron" => "\u{0160}",
        "scaron" => "\u{0161}",
        "sbquo" => "\u{201A}",
        "bdquo" => "\u{201E}",
        "lsaquo" => "\u{2039}",
        "rsaquo" => "\u{203A}",
        "oline" => "\u{203E}",
        "frasl" => "\u{2044}",
        "weierp" => "\u{2118}",
        "image" => "\u{2111}",
        "real" => "\u{211C}",
        "alefsym" => "\u{2135}",
        "crarr" => "\u{21B5}",
        "lArr" => "\u{21D0}",
        "uArr" => "\u{21D1}",
        "rArr" => "\u{21D2}",
        "dArr" => "\u{21D3}",
        "hArr" => "\u{21D4}",
        "forall" => "\u{2200}",
        "part" => "\u{2202}",
        "exist" => "\u{2203}",
        "empty" => "\u{2205}",
        "nabla" => "\u{2207}",
        "isin" => "\u{2208}",
        "notin" => "\u{2209}",
        "ni" => "\u{220B}",
        "prop" => "\u{221D}",
        "ang" => "\u{2220}",
        "and" => "\u{2227}",
        "or" => "\u{2228}",
        "cap" => "\u{2229}",
        "cup" => "\u{222A}",
        "int" => "\u{222B}",
        "there4" => "\u{2234}",
        "cong" => "\u{2245}",
        "sub" => "\u{2282}",
        "sup" => "\u{2283}",
        "nsub" => "\u{2284}",
        "sube" => "\u{2286}",
        "supe" => "\u{2287}",
        "oplus" => "\u{2295}",
        "otimes" => "\u{2297}",
        "perp" => "\u{22A5}",
        "sdot" => "\u{22C5}",
        "lceil" => "\u{2308}",
        "rceil" => "\u{2309}",
        "lfloor" => "\u{230A}",
        "rfloor" => "\u{230B}",
        "lang" => "\u{27E8}",
        "rang" => "\u{27E9}",
        "loz" => "\u{25CA}",
        "spades" => "\u{2660}",
        "clubs" => "\u{2663}",
        "hearts" => "\u{2665}",
        "diams" => "\u{2666}",

        // ---- Common HTML5 additions ----------------------------------------
        "check" => "\u{2713}",
        "cross" => "\u{2717}",
        "starf" => "\u{2605}",
        "star" => "\u{2606}",
        "hyphen" => "\u{2010}",
        "dash" => "\u{2010}",
        "ZeroWidthSpace" => "\u{200B}",
        "Tab" => "\t",
        "NewLine" => "\n",
        "excl" => "!",
        "num" => "#",
        "dollar" => "$",
        "percnt" => "%",
        "lpar" => "(",
        "rpar" => ")",
        "ast" => "*",
        "plus" => "+",
        "comma" => ",",
        "period" => ".",
        "sol" => "/",
        "colon" => ":",
        "semi" => ";",
        "equals" => "=",
        "quest" => "?",
        "commat" => "@",
        "lsqb" => "[",
        "lbrack" => "[",
        "bsol" => "\\",
        "rsqb" => "]",
        "rbrack" => "]",
        "Hat" => "^",
        "lowbar" => "_",
        "grave" => "`",
        "lcub" => "{",
        "lbrace" => "{",
        "verbar" => "|",
        "vert" => "|",
        "rcub" => "}",
        "rbrace" => "}",

        _ => return None,
    };
    Some(s)
}

/// Whether `name` is one of the legacy references the HTML spec also
/// recognises without a trailing `;` (`&copy 2024`, `&nbsp`): the
/// Latin-1 set plus `amp`, `lt`, `gt`, `quot` and their uppercase forms.
pub fn is_legacy_entity(name: &str) -> bool {
    matches!(
        name,
        "AElig"
            | "AMP"
            | "Aacute"
            | "Acirc"
            | "Agrave"
            | "Aring"
            | "Atilde"
            | "Auml"
            | "COPY"
            | "Ccedil"
            | "ETH"
            | "Eacute"
            | "Ecirc"
            | "Egrave"
            | "Euml"
            | "GT"
            | "Iacute"
            | "Icirc"
            | "Igrave"
            | "Iuml"
            | "LT"
            | "Ntilde"
            | "Oacute"
            | "Ocirc"
            | "Ograve"
            | "Oslash"
            | "Otilde"
            | "Ouml"
            | "QUOT"
            | "REG"
            | "THORN"
            | "Uacute"
            | "Ucirc"
            | "Ugrave"
            | "Uuml"
            | "Yacute"
            | "aacute"
            | "acirc"
            | "acute"
            | "aelig"
            | "agrave"
            | "amp"
            | "aring"
            | "atilde"
            | "auml"
            | "brvbar"
            | "ccedil"
            | "cedil"
            | "cent"
            | "copy"
            | "curren"
            | "deg"
            | "divide"
            | "eacute"
            | "ecirc"
            | "egrave"
            | "eth"
            | "euml"
            | "frac12"
            | "frac14"
            | "frac34"
            | "gt"
            | "iacute"
            | "icirc"
            | "iexcl"
            | "igrave"
            | "iquest"
            | "iuml"
            | "laquo"
            | "lt"
            | "macr"
            | "micro"
            | "middot"
            | "nbsp"
            | "not"
            | "ntilde"
            | "oacute"
            | "ocirc"
            | "ograve"
            | "ordf"
            | "ordm"
            | "oslash"
            | "otilde"
            | "ouml"
            | "para"
            | "plusmn"
            | "pound"
            | "quot"
            | "raquo"
            | "reg"
            | "sect"
            | "shy"
            | "sup1"
            | "sup2"
            | "sup3"
            | "szlig"
            | "thorn"
            | "times"
            | "uacute"
            | "ucirc"
            | "ugrave"
            | "uml"
            | "uuml"
            | "yacute"
            | "yen"
            | "yuml"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_entities_all_resolve() {
        for name in [
            "amp", "nbsp", "copy", "not", "frac34", "AElig", "yuml", "QUOT",
        ] {
            assert!(is_legacy_entity(name), "{name}");
            assert!(lookup_entity(name).is_some(), "{name}");
        }
        assert!(!is_legacy_entity("mdash"));
    }

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
        // Non-existent casing returns None (`AMP` and `LT` do exist).
        assert_eq!(lookup_entity("Amp"), None);
        assert_eq!(lookup_entity("lT"), None);
    }

    #[test]
    fn unknown_entity_returns_none() {
        assert_eq!(lookup_entity(""), None);
        assert_eq!(lookup_entity("notareal"), None);
        assert_eq!(lookup_entity("foobar"), None);
    }
}
