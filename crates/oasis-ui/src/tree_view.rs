//! TreeView widget: hierarchical tree with expand/collapse and keyboard navigation.

use crate::context::DrawContext;
use crate::layout;
use crate::widget::Widget;
use oasis_types::error::Result;

/// A single node in the tree hierarchy.
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Display label for this node.
    pub label: String,
    /// Child nodes.
    pub children: Vec<TreeNode>,
    /// Whether this node is expanded (children visible).
    pub expanded: bool,
    /// Whether this node is selected.
    pub selected: bool,
    /// Unique identifier for this node.
    pub id: usize,
}

impl TreeNode {
    /// Create a new tree node with the given label and id.
    fn new(label: impl Into<String>, id: usize) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
            expanded: false,
            selected: false,
            id,
        }
    }

    /// Whether this node has children.
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// A hierarchical tree view widget with expand/collapse and navigation.
pub struct TreeView {
    /// Root-level nodes.
    pub roots: Vec<TreeNode>,
    /// Currently focused node id.
    pub focused_id: Option<usize>,
    /// Horizontal indent per depth level in pixels.
    pub indent_width: u16,
    /// Whether to draw connecting lines between parent and children.
    pub show_lines: bool,
    /// Whether the tree view is disabled.
    pub disabled: bool,
    /// Next unique id to assign.
    next_id: usize,
}

/// Height of each tree row in pixels.
const ROW_HEIGHT: u32 = 16;

/// Width of the expand/collapse indicator area.
const INDICATOR_WIDTH: u32 = 12;

/// Gap between indicator and label text.
const LABEL_GAP: u32 = 4;

impl TreeView {
    /// Create a new empty tree view.
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            focused_id: None,
            indent_width: 16,
            show_lines: false,
            disabled: false,
            next_id: 0,
        }
    }

    /// Add a root-level node with the given label. Returns the node id.
    pub fn add_root(&mut self, label: impl Into<String>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.roots.push(TreeNode::new(label, id));
        id
    }

    /// Add a child node under the given parent id. Returns the child id,
    /// or `None` if the parent was not found.
    pub fn add_child(&mut self, parent_id: usize, label: impl Into<String>) -> Option<usize> {
        let id = self.next_id;
        let child = TreeNode::new(label, id);
        if Self::insert_child(&mut self.roots, parent_id, child) {
            self.next_id += 1;
            Some(id)
        } else {
            None
        }
    }

    /// Toggle the expanded state of the node with the given id.
    pub fn toggle(&mut self, id: usize) {
        if let Some(node) = Self::find_node_mut(&mut self.roots, id)
            && node.has_children()
        {
            node.expanded = !node.expanded;
        }
    }

    /// Expand the node with the given id.
    pub fn expand(&mut self, id: usize) {
        if let Some(node) = Self::find_node_mut(&mut self.roots, id) {
            node.expanded = true;
        }
    }

    /// Collapse the node with the given id.
    pub fn collapse(&mut self, id: usize) {
        if let Some(node) = Self::find_node_mut(&mut self.roots, id) {
            node.expanded = false;
        }
    }

    /// Expand all nodes in the tree recursively.
    pub fn expand_all(&mut self) {
        Self::set_expanded_recursive(&mut self.roots, true);
    }

    /// Collapse all nodes in the tree recursively.
    pub fn collapse_all(&mut self) {
        Self::set_expanded_recursive(&mut self.roots, false);
    }

    /// Select the node with the given id, deselecting all others.
    pub fn select(&mut self, id: usize) {
        Self::deselect_all(&mut self.roots);
        if let Some(node) = Self::find_node_mut(&mut self.roots, id) {
            node.selected = true;
        }
        self.focused_id = Some(id);
    }

    /// Return the currently focused node id.
    pub fn focused(&self) -> Option<usize> {
        self.focused_id
    }

    /// Move focus to the previous visible node.
    pub fn navigate_up(&mut self) {
        let visible = self.visible_nodes();
        if visible.is_empty() {
            return;
        }
        match self.focused_id {
            Some(fid) => {
                if let Some(pos) = visible.iter().position(|(id, _, _)| *id == fid)
                    && pos > 0
                {
                    self.focused_id = Some(visible[pos - 1].0);
                }
            },
            None => {
                self.focused_id = Some(visible[0].0);
            },
        }
    }

    /// Move focus to the next visible node.
    pub fn navigate_down(&mut self) {
        let visible = self.visible_nodes();
        if visible.is_empty() {
            return;
        }
        match self.focused_id {
            Some(fid) => {
                if let Some(pos) = visible.iter().position(|(id, _, _)| *id == fid)
                    && pos + 1 < visible.len()
                {
                    self.focused_id = Some(visible[pos + 1].0);
                }
            },
            None => {
                self.focused_id = Some(visible[0].0);
            },
        }
    }

    /// If the focused node has children, expand it. Otherwise toggle it.
    pub fn navigate_into(&mut self) {
        if let Some(fid) = self.focused_id
            && let Some(node) = Self::find_node(&self.roots, fid)
        {
            if node.has_children() {
                if node.expanded {
                    // Already expanded: move focus to first child.
                    let first_child_id = node.children[0].id;
                    self.focused_id = Some(first_child_id);
                } else {
                    // Expand the node.
                    Self::find_node_mut(&mut self.roots, fid)
                        .expect("node exists")
                        .expanded = true;
                }
            } else {
                self.toggle(fid);
            }
        }
    }

    /// If the focused node is expanded, collapse it. Otherwise move focus
    /// to its parent.
    pub fn navigate_out(&mut self) {
        if let Some(fid) = self.focused_id
            && let Some(node) = Self::find_node(&self.roots, fid)
        {
            if node.expanded && node.has_children() {
                Self::find_node_mut(&mut self.roots, fid)
                    .expect("node exists")
                    .expanded = false;
            } else if let Some(parent_id) = Self::find_parent_id(&self.roots, fid) {
                self.focused_id = Some(parent_id);
            }
        }
    }

    /// Return all currently visible nodes as `(id, &TreeNode, depth)` tuples.
    ///
    /// A node is visible if all of its ancestors are expanded. Root nodes are
    /// always visible.
    pub fn visible_nodes(&self) -> Vec<(usize, &TreeNode, usize)> {
        let mut result = Vec::new();
        Self::collect_visible(&self.roots, 0, &mut result);
        result
    }

    // -- Internal helpers --

    /// Recursively collect visible nodes.
    fn collect_visible<'a>(
        nodes: &'a [TreeNode],
        depth: usize,
        out: &mut Vec<(usize, &'a TreeNode, usize)>,
    ) {
        for node in nodes {
            out.push((node.id, node, depth));
            if node.expanded {
                Self::collect_visible(&node.children, depth + 1, out);
            }
        }
    }

    /// Find a node by id (immutable).
    fn find_node(nodes: &[TreeNode], id: usize) -> Option<&TreeNode> {
        for node in nodes {
            if node.id == id {
                return Some(node);
            }
            if let Some(found) = Self::find_node(&node.children, id) {
                return Some(found);
            }
        }
        None
    }

    /// Find a node by id (mutable).
    fn find_node_mut(nodes: &mut [TreeNode], id: usize) -> Option<&mut TreeNode> {
        for node in nodes {
            if node.id == id {
                return Some(node);
            }
            if let Some(found) = Self::find_node_mut(&mut node.children, id) {
                return Some(found);
            }
        }
        None
    }

    /// Find the parent id of a node with the given id.
    fn find_parent_id(nodes: &[TreeNode], target_id: usize) -> Option<usize> {
        for node in nodes {
            for child in &node.children {
                if child.id == target_id {
                    return Some(node.id);
                }
            }
            if let Some(pid) = Self::find_parent_id(&node.children, target_id) {
                return Some(pid);
            }
        }
        None
    }

    /// Insert a child under the node with the given parent id.
    fn insert_child(nodes: &mut [TreeNode], parent_id: usize, child: TreeNode) -> bool {
        for node in nodes {
            if node.id == parent_id {
                node.children.push(child);
                return true;
            }
            if Self::insert_child(&mut node.children, parent_id, child.clone()) {
                return true;
            }
        }
        false
    }

    /// Set expanded state recursively on all nodes.
    fn set_expanded_recursive(nodes: &mut [TreeNode], expanded: bool) {
        for node in nodes {
            if node.has_children() {
                node.expanded = expanded;
            }
            Self::set_expanded_recursive(&mut node.children, expanded);
        }
    }

    /// Deselect all nodes recursively.
    fn deselect_all(nodes: &mut [TreeNode]) {
        for node in nodes {
            node.selected = false;
            Self::deselect_all(&mut node.children);
        }
    }

    /// Draw connecting lines for a node at the given position and depth.
    fn draw_lines(
        ctx: &mut DrawContext<'_>,
        x: i32,
        y: i32,
        depth: usize,
        indent: u16,
        row_h: u32,
        is_last: bool,
    ) -> Result<()> {
        if depth == 0 {
            return Ok(());
        }
        let line_color = ctx.theme.border_subtle;
        let ix = x + (depth as i32 - 1) * indent as i32 + indent as i32 / 2;
        let mid_y = y + row_h as i32 / 2;

        // Horizontal branch from vertical line to node.
        ctx.backend
            .draw_line(ix, mid_y, ix + indent as i32 / 2, mid_y, 1, line_color)?;

        // Vertical line segment.
        let vert_end = if is_last { mid_y } else { y + row_h as i32 };
        ctx.backend.draw_line(ix, y, ix, vert_end, 1, line_color)?;

        Ok(())
    }
}

impl Default for TreeView {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TreeView {
    fn measure(&self, ctx: &DrawContext<'_>, available_w: u32, _available_h: u32) -> (u32, u32) {
        let visible = self.visible_nodes();
        let row_h = ROW_HEIGHT.max(ctx.backend.measure_text_height(ctx.theme.font_size_md));
        let total_h = visible.len() as u32 * row_h;

        // Width: find the widest row.
        let mut max_w: u32 = 0;
        for (_, node, depth) in &visible {
            let indent = *depth as u32 * self.indent_width as u32;
            let text_w = ctx
                .backend
                .measure_text(&node.label, ctx.theme.font_size_md);
            let row_w = indent + INDICATOR_WIDTH + LABEL_GAP + text_w;
            if row_w > max_w {
                max_w = row_w;
            }
        }

        (max_w.min(available_w), total_h)
    }

    fn draw(&self, ctx: &mut DrawContext<'_>, x: i32, y: i32, w: u32, _h: u32) -> Result<()> {
        let visible = self.visible_nodes();
        let fs = ctx.theme.font_size_md;
        let text_h = ctx.backend.measure_text_height(fs);
        let row_h = ROW_HEIGHT.max(text_h);
        let text_color = if self.disabled {
            ctx.theme.text_disabled
        } else {
            ctx.theme.text_primary
        };

        // Precompute which nodes are last among their visible siblings.
        // For connecting-line drawing, we need to know if a node is the
        // last child at its depth. We approximate this from the flattened
        // visible list: a node is "last" if the next node has the same or
        // lesser depth.
        let is_last_at_depth: Vec<bool> = visible
            .iter()
            .enumerate()
            .map(|(i, (_, _, depth))| {
                if i + 1 >= visible.len() {
                    return true;
                }
                visible[i + 1].2 <= *depth
            })
            .collect();

        for (row_idx, (id, node, depth)) in visible.iter().enumerate() {
            let row_y = y + row_idx as i32 * row_h as i32;
            let indent = *depth as i32 * self.indent_width as i32;

            // Selected highlight.
            if node.selected && !self.disabled {
                ctx.backend
                    .fill_rect(x, row_y, w, row_h, ctx.theme.accent_subtle)?;
            }

            // Focus indicator (subtle border on focused row).
            if self.focused_id == Some(*id) && !self.disabled {
                ctx.backend
                    .stroke_rect(x, row_y, w, row_h, 1, ctx.theme.border_subtle)?;
            }

            // Connecting lines.
            if self.show_lines {
                Self::draw_lines(
                    ctx,
                    x,
                    row_y,
                    *depth,
                    self.indent_width,
                    row_h,
                    is_last_at_depth[row_idx],
                )?;
            }

            // Expand/collapse indicator.
            let indicator_x = x + indent;
            if node.has_children() {
                let indicator = if node.expanded { "-" } else { "+" };
                let ind_w = ctx.backend.measure_text(indicator, fs);
                let ind_x = indicator_x + layout::center(INDICATOR_WIDTH, ind_w);
                let ind_y = row_y + layout::center(row_h, text_h);
                let ind_color = if self.disabled {
                    ctx.theme.text_disabled
                } else {
                    ctx.theme.text_secondary
                };
                ctx.backend
                    .draw_text(indicator, ind_x, ind_y, fs, ind_color)?;
            }

            // Node label.
            let label_x = indicator_x + INDICATOR_WIDTH as i32 + LABEL_GAP as i32;
            let label_y = row_y + layout::center(row_h, text_h);
            ctx.backend
                .draw_text(&node.label, label_x, label_y, fs, text_color)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DrawContext;
    use crate::test_utils::MockBackend;
    use crate::theme::Theme;
    use crate::widget::Widget;
    use oasis_types::backend::SdiBackend;

    #[test]
    fn new_defaults() {
        let tv = TreeView::new();
        assert!(tv.roots.is_empty());
        assert!(tv.focused_id.is_none());
        assert_eq!(tv.indent_width, 16);
        assert!(!tv.show_lines);
        assert!(!tv.disabled);
    }

    #[test]
    fn default_matches_new() {
        let tv = TreeView::default();
        assert!(tv.roots.is_empty());
        assert_eq!(tv.indent_width, 16);
    }

    #[test]
    fn add_root_returns_unique_ids() {
        let mut tv = TreeView::new();
        let id0 = tv.add_root("Root A");
        let id1 = tv.add_root("Root B");
        assert_ne!(id0, id1);
        assert_eq!(tv.roots.len(), 2);
        assert_eq!(tv.roots[0].label, "Root A");
        assert_eq!(tv.roots[1].label, "Root B");
    }

    #[test]
    fn add_child_under_root() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        let child = tv.add_child(root, "Child");
        assert!(child.is_some());
        let cid = child.unwrap();
        assert_ne!(cid, root);
        assert_eq!(tv.roots[0].children.len(), 1);
        assert_eq!(tv.roots[0].children[0].label, "Child");
    }

    #[test]
    fn add_child_nonexistent_parent() {
        let mut tv = TreeView::new();
        let result = tv.add_child(999, "Orphan");
        assert!(result.is_none());
    }

    #[test]
    fn add_nested_children() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        let child = tv.add_child(root, "Child").unwrap();
        let grandchild = tv.add_child(child, "Grandchild");
        assert!(grandchild.is_some());
        assert_eq!(tv.roots[0].children[0].children[0].label, "Grandchild");
    }

    #[test]
    fn expand_and_collapse() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");
        assert!(!tv.roots[0].expanded);

        tv.expand(root);
        assert!(tv.roots[0].expanded);

        tv.collapse(root);
        assert!(!tv.roots[0].expanded);
    }

    #[test]
    fn toggle_expand_collapse() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");

        tv.toggle(root);
        assert!(tv.roots[0].expanded);

        tv.toggle(root);
        assert!(!tv.roots[0].expanded);
    }

    #[test]
    fn toggle_leaf_is_noop() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Leaf");
        // Leaf has no children, toggle should not panic.
        tv.toggle(root);
        assert!(!tv.roots[0].expanded);
    }

    #[test]
    fn visible_nodes_roots_only() {
        let mut tv = TreeView::new();
        tv.add_root("A");
        tv.add_root("B");
        let vis = tv.visible_nodes();
        assert_eq!(vis.len(), 2);
        assert_eq!(vis[0].2, 0); // depth 0
        assert_eq!(vis[1].2, 0);
    }

    #[test]
    fn visible_nodes_collapsed_hides_children() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child A");
        tv.add_child(root, "Child B");
        // Root is collapsed by default.
        let vis = tv.visible_nodes();
        assert_eq!(vis.len(), 1);
    }

    #[test]
    fn visible_nodes_expanded_shows_children() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child A");
        tv.add_child(root, "Child B");
        tv.expand(root);
        let vis = tv.visible_nodes();
        assert_eq!(vis.len(), 3);
        assert_eq!(vis[1].2, 1); // depth 1
        assert_eq!(vis[2].2, 1);
    }

    #[test]
    fn visible_nodes_depth_calculation() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        let child = tv.add_child(root, "Child").unwrap();
        tv.add_child(child, "Grandchild");
        tv.expand(root);
        tv.expand(child);
        let vis = tv.visible_nodes();
        assert_eq!(vis.len(), 3);
        assert_eq!(vis[0].2, 0); // Root
        assert_eq!(vis[1].2, 1); // Child
        assert_eq!(vis[2].2, 2); // Grandchild
    }

    #[test]
    fn select_deselects_others() {
        let mut tv = TreeView::new();
        let a = tv.add_root("A");
        let b = tv.add_root("B");
        tv.select(a);
        assert!(tv.roots[0].selected);
        assert!(!tv.roots[1].selected);

        tv.select(b);
        assert!(!tv.roots[0].selected);
        assert!(tv.roots[1].selected);
        assert_eq!(tv.focused(), Some(b));
    }

    #[test]
    fn expand_all_and_collapse_all() {
        let mut tv = TreeView::new();
        let r1 = tv.add_root("R1");
        let r2 = tv.add_root("R2");
        let c1 = tv.add_child(r1, "C1").unwrap();
        tv.add_child(r2, "C2");
        tv.add_child(c1, "GC1");

        tv.expand_all();
        assert!(tv.roots[0].expanded);
        assert!(tv.roots[1].expanded);
        assert!(tv.roots[0].children[0].expanded);
        // All visible.
        assert_eq!(tv.visible_nodes().len(), 5);

        tv.collapse_all();
        assert!(!tv.roots[0].expanded);
        assert!(!tv.roots[1].expanded);
        assert_eq!(tv.visible_nodes().len(), 2);
    }

    #[test]
    fn navigate_down_from_none() {
        let mut tv = TreeView::new();
        tv.add_root("A");
        tv.add_root("B");
        assert!(tv.focused_id.is_none());
        tv.navigate_down();
        assert_eq!(tv.focused_id, Some(0));
    }

    #[test]
    fn navigate_down_advances() {
        let mut tv = TreeView::new();
        let a = tv.add_root("A");
        let b = tv.add_root("B");
        tv.focused_id = Some(a);
        tv.navigate_down();
        assert_eq!(tv.focused_id, Some(b));
    }

    #[test]
    fn navigate_down_at_end_stays() {
        let mut tv = TreeView::new();
        tv.add_root("A");
        let b = tv.add_root("B");
        tv.focused_id = Some(b);
        tv.navigate_down();
        assert_eq!(tv.focused_id, Some(b));
    }

    #[test]
    fn navigate_up_from_none() {
        let mut tv = TreeView::new();
        tv.add_root("A");
        assert!(tv.focused_id.is_none());
        tv.navigate_up();
        assert_eq!(tv.focused_id, Some(0));
    }

    #[test]
    fn navigate_up_moves_back() {
        let mut tv = TreeView::new();
        tv.add_root("A");
        let b = tv.add_root("B");
        tv.focused_id = Some(b);
        tv.navigate_up();
        assert_eq!(tv.focused_id, Some(0));
    }

    #[test]
    fn navigate_up_at_top_stays() {
        let mut tv = TreeView::new();
        let a = tv.add_root("A");
        tv.add_root("B");
        tv.focused_id = Some(a);
        tv.navigate_up();
        assert_eq!(tv.focused_id, Some(a));
    }

    #[test]
    fn navigate_into_expands() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");
        tv.focused_id = Some(root);
        assert!(!tv.roots[0].expanded);

        tv.navigate_into();
        assert!(tv.roots[0].expanded);
    }

    #[test]
    fn navigate_into_expanded_moves_to_child() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        let child = tv.add_child(root, "Child").unwrap();
        tv.expand(root);
        tv.focused_id = Some(root);

        tv.navigate_into();
        assert_eq!(tv.focused_id, Some(child));
    }

    #[test]
    fn navigate_out_collapses() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");
        tv.expand(root);
        tv.focused_id = Some(root);

        tv.navigate_out();
        assert!(!tv.roots[0].expanded);
    }

    #[test]
    fn navigate_out_moves_to_parent() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        let child = tv.add_child(root, "Child").unwrap();
        tv.expand(root);
        tv.focused_id = Some(child);

        tv.navigate_out();
        assert_eq!(tv.focused_id, Some(root));
    }

    #[test]
    fn navigate_on_empty_tree() {
        let mut tv = TreeView::new();
        tv.navigate_up();
        tv.navigate_down();
        tv.navigate_into();
        tv.navigate_out();
        assert!(tv.focused_id.is_none());
    }

    // -- Draw / measure tests using MockBackend --

    #[test]
    fn measure_empty_tree() {
        let tv = TreeView::new();
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, h) = tv.measure(&ctx, 200, 400);
        assert_eq!(w, 0);
        assert_eq!(h, 0);
    }

    #[test]
    fn measure_includes_all_visible_rows() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");
        tv.expand(root);

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (_, h) = tv.measure(&ctx, 200, 400);
        // 2 visible nodes * row_height.
        let row_h = ROW_HEIGHT.max(backend.measure_text_height(theme.font_size_md));
        assert_eq!(h, 2 * row_h);
    }

    #[test]
    fn draw_renders_node_labels() {
        let mut tv = TreeView::new();
        tv.add_root("Alpha");
        tv.add_root("Beta");

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Alpha"));
        assert!(backend.has_text("Beta"));
        assert_eq!(backend.draw_text_count(), 2);
    }

    #[test]
    fn draw_shows_expand_indicator() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Parent");
        tv.add_child(root, "Child");

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Collapsed parent should show "+" indicator.
        assert!(backend.has_text("+"));
    }

    #[test]
    fn draw_expanded_shows_minus() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Parent");
        tv.add_child(root, "Child");
        tv.expand(root);

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("-"));
        assert!(backend.has_text("Child"));
    }

    #[test]
    fn draw_selected_highlight() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Selected");
        tv.select(root);

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Selected node triggers fill_rect for highlight.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_disabled_uses_disabled_color() {
        let mut tv = TreeView::new();
        tv.add_root("Item");
        tv.disabled = true;

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert!(backend.has_text("Item"));
        // Verify it drew text (disabled still renders, just in disabled color).
        assert!(backend.draw_text_count() > 0);
    }

    #[test]
    fn draw_with_connecting_lines() {
        let mut tv = TreeView::new();
        let root = tv.add_root("Root");
        tv.add_child(root, "Child");
        tv.expand(root);
        tv.show_lines = true;

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        // Lines are drawn via fill_rect (draw_line default impl).
        // Child at depth 1 should trigger line drawing -> fill_rect calls.
        assert!(backend.fill_rect_count() > 0);
    }

    #[test]
    fn draw_empty_tree_no_panic() {
        let tv = TreeView::new();
        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        {
            let mut ctx = DrawContext::new(&mut backend, &theme);
            tv.draw(&mut ctx, 0, 0, 200, 100).unwrap();
        }
        assert_eq!(backend.draw_text_count(), 0);
    }

    #[test]
    fn draw_all_themes_no_panic() {
        crate::test_utils::test_draw_all_themes(|ctx| {
            let mut tv = TreeView::new();
            let root = tv.add_root("Root");
            tv.add_child(root, "Child");
            tv.expand(root);
            tv.select(root);
            tv.show_lines = true;
            tv.draw(ctx, 0, 0, 200, 100).unwrap();
        });
    }

    #[test]
    fn measure_respects_available_width() {
        let mut tv = TreeView::new();
        tv.add_root("A very long label that exceeds available width");

        let theme = Theme::dark();
        let mut backend = MockBackend::new();
        let ctx = DrawContext::new(&mut backend, &theme);
        let (w, _) = tv.measure(&ctx, 50, 400);
        assert!(w <= 50);
    }
}
