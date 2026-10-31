//! `sdi list` / `sdi get <name>`: render the scene graph as terminal text.

use crate::backend::Color;
use crate::sdi::SdiRegistry;

fn fmt_color(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
}

/// Render the `sdi list` / `sdi get <name>` terminal output for `sdi`.
///
/// `None` lists every object name (z-ordered, hidden ones marked);
/// `Some(name)` prints all of that object's fields.
pub fn inspect_sdi(sdi: &SdiRegistry, name: Option<&str>) -> String {
    use std::fmt::Write as _;
    let Some(name) = name else {
        let mut objs: Vec<_> = sdi.names().filter_map(|n| sdi.get(n).ok()).collect();
        objs.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.name.cmp(&b.name)));
        let mut out = format!("{} SDI objects:", objs.len());
        for o in objs {
            let hidden = if o.visible { "" } else { "  (hidden)" };
            let _ = write!(out, "\n  {:<28} z={}{hidden}", o.name, o.z);
        }
        return out;
    };
    let Ok(o) = sdi.get(name) else {
        return format!("sdi: no object named '{name}'");
    };
    let mut out = format!("{}:", o.name);
    let _ = write!(
        out,
        "\n  pos      {}, {}\n  size     {} x {}\n  z        {}\n  visible  {}\
         \n  overlay  {}\n  alpha    {}\n  color    {}",
        o.x,
        o.y,
        o.w,
        o.h,
        o.z,
        o.visible,
        o.overlay,
        o.alpha,
        fmt_color(o.color)
    );
    if let Some(ref text) = o.text {
        let _ = write!(
            out,
            "\n  text     {text:?}\n  font     {}px\n  text_color {}",
            o.font_size,
            fmt_color(o.text_color)
        );
    }
    if let Some(tex) = o.texture {
        let _ = write!(out, "\n  texture  {}", tex.0);
    }
    if o.nine_patch.is_some() {
        out.push_str("\n  nine_patch yes");
    }
    if let Some(r) = o.border_radius {
        let _ = write!(out, "\n  radius   {r}");
    }
    if let (Some(top), Some(bottom)) = (o.gradient_top, o.gradient_bottom) {
        let _ = write!(
            out,
            "\n  gradient {} -> {}",
            fmt_color(top),
            fmt_color(bottom)
        );
    }
    if let Some(w) = o.stroke_width {
        let color = o.stroke_color.map_or_else(|| "-".to_string(), fmt_color);
        let _ = write!(out, "\n  stroke   {w}px {color}");
    }
    if let Some(level) = o.shadow_level {
        let _ = write!(out, "\n  shadow   level {level}");
    }
    if let Some((dx, dy)) = o.text_shadow_offset {
        let _ = write!(out, "\n  text_shadow {dx}, {dy}");
    }
    if let Some(ref label) = o.aria_label {
        let _ = write!(out, "\n  aria     {label:?}");
    }
    out
}

/// Replace every [`CommandSignal::SdiInspect`] in a command result with
/// the text [`inspect_sdi`] renders, so hosts can print it like any
/// other output.
///
/// [`CommandSignal::SdiInspect`]: crate::terminal::CommandSignal::SdiInspect
pub fn resolve_sdi_inspect(
    output: crate::terminal::CommandOutput,
    sdi: &SdiRegistry,
) -> crate::terminal::CommandOutput {
    use crate::terminal::{CommandOutput, CommandSignal};
    match output {
        CommandOutput::Signal(CommandSignal::SdiInspect { name }) => {
            CommandOutput::Text(inspect_sdi(sdi, name.as_deref()))
        },
        CommandOutput::Multi(outputs) => CommandOutput::Multi(
            outputs
                .into_iter()
                .map(|o| resolve_sdi_inspect(o, sdi))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_sdi_lists_and_describes_objects() {
        let mut sdi = SdiRegistry::new();
        {
            let obj = sdi.create("label");
            obj.x = 10;
            obj.y = 20;
            obj.w = 30;
            obj.h = 40;
            obj.text = Some("hello".to_string());
            obj.color = Color::rgb(255, 0, 0);
        }
        sdi.create("hidden_box").visible = false;

        let list = inspect_sdi(&sdi, None);
        assert!(list.starts_with("2 SDI objects:"), "{list}");
        assert!(list.contains("hidden_box"));
        assert!(list.contains("(hidden)"));

        let desc = inspect_sdi(&sdi, Some("label"));
        assert!(desc.starts_with("label:"), "{desc}");
        assert!(desc.contains("pos      10, 20"), "{desc}");
        assert!(desc.contains("size     30 x 40"), "{desc}");
        assert!(desc.contains("color    #ff0000ff"), "{desc}");
        assert!(desc.contains("text     \"hello\""), "{desc}");

        assert!(inspect_sdi(&sdi, Some("nope")).contains("no object named 'nope'"));

        use crate::terminal::{CommandOutput, CommandSignal};
        let resolved = resolve_sdi_inspect(
            CommandOutput::Multi(vec![CommandOutput::Signal(CommandSignal::SdiInspect {
                name: Some("label".to_string()),
            })]),
            &sdi,
        );
        let CommandOutput::Multi(inner) = resolved else {
            panic!("expected Multi");
        };
        assert!(matches!(&inner[0], CommandOutput::Text(t) if t.starts_with("label:")));
    }
}
