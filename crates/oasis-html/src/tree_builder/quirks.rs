//! Quirks-mode selection from the DOCTYPE (WHATWG §13.2.6.4.1, "the
//! initial insertion mode").

use super::super::dom::QuirksMode;
use super::super::tokenizer::DoctypeToken;

/// Public identifier prefixes that select quirks mode.
const QUIRKS_PUBLIC_PREFIXES: &[&str] = &[
    "+//silmaril//dtd html pro v0r11 19970101//",
    "-//as//dtd html 3.0 aswedit + extensions//",
    "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
    "-//ietf//dtd html 2.0 level 1//",
    "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//",
    "-//ietf//dtd html 2.0 strict level 2//",
    "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//",
    "-//ietf//dtd html 2.1e//",
    "-//ietf//dtd html 3.0//",
    "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//",
    "-//ietf//dtd html 3//",
    "-//ietf//dtd html level 0//",
    "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//",
    "-//ietf//dtd html level 3//",
    "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//",
    "-//ietf//dtd html strict level 2//",
    "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//",
    "-//ietf//dtd html//",
    "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//",
    "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//",
    "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//",
    "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//",
    "-//netscape comm. corp.//dtd strict html//",
    "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//",
    "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
    "-//spyglass//dtd html 2.0 extended//",
    "-//sun microsystems corp.//dtd hotjava html//",
    "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//",
    "-//w3c//dtd html 3.2 draft//",
    "-//w3c//dtd html 3.2 final//",
    "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//",
    "-//w3c//dtd html 4.0 frameset//",
    "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//",
    "-//w3c//dtd html experimental 970421//",
    "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//",
    "-//webtechs//dtd mozilla html 2.0//",
    "-//webtechs//dtd mozilla html//",
];

/// HTML 4.01 frameset / transitional: quirks without a system
/// identifier, limited quirks with one.
const HTML401_LOOSE_PREFIXES: &[&str] = &[
    "-//w3c//dtd html 4.01 frameset//",
    "-//w3c//dtd html 4.01 transitional//",
];

/// XHTML 1.0 frameset / transitional: limited quirks.
const XHTML_LOOSE_PREFIXES: &[&str] = &[
    "-//w3c//dtd xhtml 1.0 frameset//",
    "-//w3c//dtd xhtml 1.0 transitional//",
];

/// The compatibility mode a DOCTYPE token selects.
pub(crate) fn quirks_mode_for(doctype: &DoctypeToken) -> QuirksMode {
    let public = doctype.public_id.as_deref().map(str::to_ascii_lowercase);
    let system = doctype.system_id.as_deref().map(str::to_ascii_lowercase);
    let public = public.as_deref();
    let starts =
        |prefixes: &[&str]| public.is_some_and(|p| prefixes.iter().any(|x| p.starts_with(x)));
    let quirks = doctype.force_quirks
        || doctype.name.as_deref() != Some("html")
        || matches!(
            public,
            Some(
                "-//w3o//dtd w3 html strict 3.0//en//"
                    | "-/w3c/dtd html 4.0 transitional/en"
                    | "html"
            )
        )
        || system.as_deref() == Some("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
        || starts(QUIRKS_PUBLIC_PREFIXES)
        || (system.is_none() && starts(HTML401_LOOSE_PREFIXES));
    if quirks {
        QuirksMode::Quirks
    } else if starts(XHTML_LOOSE_PREFIXES) || (system.is_some() && starts(HTML401_LOOSE_PREFIXES)) {
        QuirksMode::LimitedQuirks
    } else {
        QuirksMode::NoQuirks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doctype(name: &str, public: Option<&str>, system: Option<&str>) -> DoctypeToken {
        DoctypeToken {
            name: Some(name.to_string()),
            public_id: public.map(str::to_string),
            system_id: system.map(str::to_string),
            force_quirks: false,
        }
    }

    #[test]
    fn html5_doctype_is_no_quirks() {
        assert_eq!(
            quirks_mode_for(&doctype("html", None, None)),
            QuirksMode::NoQuirks
        );
    }

    #[test]
    fn force_quirks_and_other_names_are_quirks() {
        let mut d = doctype("html", None, None);
        d.force_quirks = true;
        assert_eq!(quirks_mode_for(&d), QuirksMode::Quirks);
        assert_eq!(
            quirks_mode_for(&doctype("svg", None, None)),
            QuirksMode::Quirks
        );
    }

    #[test]
    fn legacy_public_ids_are_quirks() {
        let d = doctype("html", Some("-//W3C//DTD HTML 3.2 Final//EN"), None);
        assert_eq!(quirks_mode_for(&d), QuirksMode::Quirks);
        let d = doctype("html", Some("-//W3C//DTD HTML 4.01 Transitional//EN"), None);
        assert_eq!(quirks_mode_for(&d), QuirksMode::Quirks);
    }

    #[test]
    fn transitional_with_system_id_is_limited_quirks() {
        let d = doctype(
            "html",
            Some("-//W3C//DTD HTML 4.01 Transitional//EN"),
            Some("http://www.w3.org/TR/html4/loose.dtd"),
        );
        assert_eq!(quirks_mode_for(&d), QuirksMode::LimitedQuirks);
        let d = doctype("html", Some("-//W3C//DTD XHTML 1.0 Transitional//EN"), None);
        assert_eq!(quirks_mode_for(&d), QuirksMode::LimitedQuirks);
    }

    #[test]
    fn strict_doctypes_are_no_quirks() {
        let d = doctype(
            "html",
            Some("-//W3C//DTD HTML 4.01//EN"),
            Some("http://www.w3.org/TR/html4/strict.dtd"),
        );
        assert_eq!(quirks_mode_for(&d), QuirksMode::NoQuirks);
        let d = doctype("html", None, Some("about:legacy-compat"));
        assert_eq!(quirks_mode_for(&d), QuirksMode::NoQuirks);
    }
}
