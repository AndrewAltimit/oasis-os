//! Cascade-layer ordering (CSS Cascade 5 §6.4.3).
//!
//! Layers form a tree: `a.b` is a sublayer of `a`, and declaring `a.b`
//! implicitly declares `a` first. Siblings are ordered by first
//! declaration, and a layer's own (sublayer-less) rules come *after*
//! all of its sublayers. So `@layer a { @layer b {..} }` orders as
//! `a.b < a`, and `@layer a; @layer b; @layer a.c {..}` as
//! `a.c < a < b` — first-appearance order gets both wrong.

use rustc_hash::FxHashMap;

use super::types::Rule;

/// Cascade rank of each of `names` (dotted full layer names, in
/// first-declaration order): a post-order walk of the layer tree, so a
/// lower rank loses to a higher one for normal declarations.
pub(crate) fn layer_cascade_ranks<S: AsRef<str>>(names: &[S]) -> Vec<u32> {
    // Node 0 is the implicit root; `children[n]` in declaration order.
    let mut children: Vec<Vec<usize>> = vec![Vec::new()];
    let mut by_name: FxHashMap<&str, usize> = FxHashMap::default();
    let mut node_of: Vec<usize> = Vec::with_capacity(names.len());
    for name in names {
        let name = name.as_ref();
        let mut parent = 0;
        // Walk every dotted prefix (`a`, `a.b`, `a.b.c`), declaring the
        // missing ones as they are reached.
        let ends = name
            .match_indices('.')
            .map(|(i, _)| i)
            .chain(std::iter::once(name.len()));
        for end in ends {
            let prefix = &name[..end];
            parent = match by_name.get(prefix) {
                Some(&node) => node,
                None => {
                    let node = children.len();
                    children.push(Vec::new());
                    children[parent].push(node);
                    by_name.insert(prefix, node);
                    node
                },
            };
        }
        node_of.push(parent);
    }

    // Iterative post-order (names can nest arbitrarily deep).
    let mut rank = vec![0u32; children.len()];
    let mut next = 0u32;
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    while let Some(&mut (node, ref mut child_idx)) = stack.last_mut() {
        if let Some(&child) = children[node].get(*child_idx) {
            *child_idx += 1;
            stack.push((child, 0));
        } else {
            rank[node] = next;
            next = next.saturating_add(1);
            stack.pop();
        }
    }
    node_of.into_iter().map(|n| rank[n]).collect()
}

/// Reorder a sheet's `layers` (first-declaration order, as the parser
/// registers them) into cascade order and remap every rule's `layer`
/// index to match, so a plain index comparison is the layer order.
pub(super) fn sort_layers_into_cascade_order(
    layers: Vec<String>,
    rules: &mut [Rule],
) -> Vec<String> {
    if layers.len() < 2 {
        return layers;
    }
    let ranks = layer_cascade_ranks(&layers);
    let mut order: Vec<usize> = (0..layers.len()).collect();
    order.sort_by_key(|&i| ranks[i]);
    // `remap[old] = new`.
    let mut remap = vec![0u16; layers.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = u16::try_from(new).unwrap_or(u16::MAX);
    }
    for rule in rules.iter_mut() {
        if let Some(old) = rule.layer {
            rule.layer = Some(remap.get(usize::from(old)).copied().unwrap_or(old));
        }
    }
    let mut slots: Vec<Option<String>> = layers.into_iter().map(Some).collect();
    order.iter().filter_map(|&old| slots[old].take()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordered(names: &[&str]) -> Vec<String> {
        let ranks = layer_cascade_ranks(names);
        let mut v: Vec<(u32, &str)> = ranks.into_iter().zip(names.iter().copied()).collect();
        v.sort();
        v.into_iter().map(|(_, n)| n.to_string()).collect()
    }

    #[test]
    fn sublayers_precede_their_parent() {
        assert_eq!(ordered(&["a", "a.b", "a.c"]), ["a.b", "a.c", "a"]);
    }

    #[test]
    fn late_sublayer_stays_inside_its_parent_slot() {
        assert_eq!(ordered(&["a", "b", "a.c"]), ["a.c", "a", "b"]);
    }

    #[test]
    fn dotted_name_declares_its_ancestors_first() {
        // `a.b` declares `a` before `c`, so `a` (and `a.b`) lose to `c`.
        assert_eq!(ordered(&["a.b", "c", "a"]), ["a.b", "a", "c"]);
    }

    #[test]
    fn deep_names_do_not_recurse() {
        let deep = vec!["x"; 20_000].join(".");
        assert_eq!(layer_cascade_ranks(&[deep.as_str()]), [0]);
    }
}
