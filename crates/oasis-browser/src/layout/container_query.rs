//! `@container` support on the layout side: snapshot every query
//! container in a laid-out tree into the cascade's [`ContainerLookup`].
//!
//! The cascade (`css`) owns the lookup type and evaluates the rule
//! conditions; this module owns the walk because it reads `LayoutBox`
//! geometry, which keeps `css` free of any dependency on `layout`.

use crate::css::cascade::{ContainerEntry, ContainerLookup};
use crate::css::values::types::ContainerType;
use crate::layout::box_model::LayoutBox;

/// Walk a laid-out `LayoutBox` tree and snapshot every query container
/// (any element whose computed `container-type` is not `Normal`), keyed
/// by its DOM node id. `@container` rule conditions are evaluated
/// against the container's content-box size.
pub fn build_container_lookup(root: &LayoutBox) -> ContainerLookup {
    fn walk(b: &LayoutBox, out: &mut ContainerLookup) {
        if let Some(nid) = b.node
            && b.style.container_type != ContainerType::Normal
        {
            out.insert(
                nid,
                ContainerEntry {
                    names: b.style.container_name.clone(),
                    width: b.dimensions.content.width,
                    height: b.dimensions.content.height,
                    container_type: b.style.container_type,
                    custom_properties: std::sync::Arc::clone(&b.style.custom_properties),
                },
            );
        }
        for c in &b.children {
            walk(c, out);
        }
    }

    let mut lookup = ContainerLookup::new();
    walk(root, &mut lookup);
    lookup
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::ComputedStyle;
    use crate::layout::box_model::{BoxType, Dimensions, Rect};

    #[test]
    fn build_container_lookup_picks_up_container_type() {
        let style = ComputedStyle {
            container_type: ContainerType::InlineSize,
            container_name: vec!["card".to_string()],
            ..Default::default()
        };

        let mut child = LayoutBox::new(BoxType::Block, style, Some(7));
        child.dimensions = Dimensions {
            content: Rect::new(0.0, 0.0, 320.0, 240.0),
            ..Dimensions::default()
        };

        let lookup = build_container_lookup(&child);
        let entry = lookup.get(7).expect("container should be indexed");
        assert_eq!(entry.names, vec!["card".to_string()]);
        assert!((entry.width - 320.0).abs() < 0.01);
        assert!((entry.height - 240.0).abs() < 0.01);
    }
}
