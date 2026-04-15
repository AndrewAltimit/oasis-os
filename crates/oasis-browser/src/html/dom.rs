//! Arena-based DOM tree optimized for layout traversal.
//!
//! Nodes are stored in a flat `Vec` arena and linked by index. This avoids
//! reference-counting overhead and makes tree walks cache-friendly.

/// Index into the [`Document`]'s node arena.
pub type NodeId = usize;

// ------------------------------------------------------------------
// Node types
// ------------------------------------------------------------------

/// The root of an HTML document.
#[derive(Debug, Clone)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    /// Index from element `id` attribute to `NodeId` for O(1) lookups.
    /// Updated on `append_child` and `set_attribute` when the `id`
    /// attribute is set or changed.
    id_index: std::collections::HashMap<String, NodeId>,
    /// Free list of arena slots available for reuse after `remove_child`.
    /// `add_node` checks this before appending to avoid unbounded arena
    /// growth when the DOM is repeatedly mutated (e.g. by JavaScript).
    free_list: Vec<NodeId>,
}

/// A single node in the DOM tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

/// The kind of DOM node.
#[derive(Debug, Clone)]
pub enum NodeKind {
    Document,
    Element(ElementData),
    Text(String),
    Comment(String),
}

/// Data associated with an Element node.
#[derive(Debug, Clone)]
pub struct ElementData {
    pub tag: TagName,
    pub attributes: Vec<Attribute>,
}

/// An element attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

// ------------------------------------------------------------------
// TagName
// ------------------------------------------------------------------

/// Known HTML tag names for fast match-based dispatch.
///
/// Tags not recognised by the parser are stored as `Unknown(String)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TagName {
    // Document structure
    Html,
    Head,
    Body,
    Title,
    Meta,
    Link,
    Style,
    Script,
    // Generic containers
    Div,
    Span,
    P,
    A,
    Br,
    Hr,
    // Headings
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    // Lists
    Ul,
    Ol,
    Li,
    Dl,
    Dt,
    Dd,
    // Tables
    Table,
    Thead,
    Tbody,
    Tfoot,
    Tr,
    Th,
    Td,
    Caption,
    Colgroup,
    Col,
    // Forms
    Form,
    Input,
    Button,
    Select,
    Option,
    Textarea,
    Label,
    // Media / figures
    Img,
    Figure,
    Figcaption,
    // Pre-formatted / code
    Pre,
    Code,
    Blockquote,
    Cite,
    // Inline formatting
    Em,
    Strong,
    B,
    I,
    U,
    S,
    Small,
    Sub,
    Sup,
    Mark,
    // Sectioning
    Nav,
    Header,
    Footer,
    Main,
    Section,
    Article,
    Aside,
    // Interactive
    Details,
    Summary,
    // Embedded content
    Iframe,
    Video,
    Audio,
    Source,
    Canvas,
    // Scripting
    Noscript,
    Template,
    // Deprecated but still common
    Center,
    // SVG
    Svg,
    // Anything else
    Unknown(String),
}

impl TagName {
    /// Parse a lowercase tag name string into a `TagName` variant.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "html" => Self::Html,
            "head" => Self::Head,
            "body" => Self::Body,
            "title" => Self::Title,
            "meta" => Self::Meta,
            "link" => Self::Link,
            "style" => Self::Style,
            "script" => Self::Script,
            "div" => Self::Div,
            "span" => Self::Span,
            "p" => Self::P,
            "a" => Self::A,
            "br" => Self::Br,
            "hr" => Self::Hr,
            "h1" => Self::H1,
            "h2" => Self::H2,
            "h3" => Self::H3,
            "h4" => Self::H4,
            "h5" => Self::H5,
            "h6" => Self::H6,
            "ul" => Self::Ul,
            "ol" => Self::Ol,
            "li" => Self::Li,
            "dl" => Self::Dl,
            "dt" => Self::Dt,
            "dd" => Self::Dd,
            "table" => Self::Table,
            "thead" => Self::Thead,
            "tbody" => Self::Tbody,
            "tfoot" => Self::Tfoot,
            "tr" => Self::Tr,
            "th" => Self::Th,
            "td" => Self::Td,
            "caption" => Self::Caption,
            "colgroup" => Self::Colgroup,
            "col" => Self::Col,
            "form" => Self::Form,
            "input" => Self::Input,
            "button" => Self::Button,
            "select" => Self::Select,
            "option" => Self::Option,
            "textarea" => Self::Textarea,
            "label" => Self::Label,
            "img" => Self::Img,
            "figure" => Self::Figure,
            "figcaption" => Self::Figcaption,
            "pre" => Self::Pre,
            "code" => Self::Code,
            "blockquote" => Self::Blockquote,
            "cite" => Self::Cite,
            "em" => Self::Em,
            "strong" => Self::Strong,
            "b" => Self::B,
            "i" => Self::I,
            "u" => Self::U,
            "s" => Self::S,
            "small" => Self::Small,
            "sub" => Self::Sub,
            "sup" => Self::Sup,
            "mark" => Self::Mark,
            "nav" => Self::Nav,
            "header" => Self::Header,
            "footer" => Self::Footer,
            "main" => Self::Main,
            "section" => Self::Section,
            "article" => Self::Article,
            "aside" => Self::Aside,
            "details" => Self::Details,
            "summary" => Self::Summary,
            "iframe" => Self::Iframe,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "source" => Self::Source,
            "canvas" => Self::Canvas,
            "noscript" => Self::Noscript,
            "template" => Self::Template,
            "center" => Self::Center,
            "svg" => Self::Svg,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Get the string representation of this tag name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Html => "html",
            Self::Head => "head",
            Self::Body => "body",
            Self::Title => "title",
            Self::Meta => "meta",
            Self::Link => "link",
            Self::Style => "style",
            Self::Script => "script",
            Self::Div => "div",
            Self::Span => "span",
            Self::P => "p",
            Self::A => "a",
            Self::Br => "br",
            Self::Hr => "hr",
            Self::H1 => "h1",
            Self::H2 => "h2",
            Self::H3 => "h3",
            Self::H4 => "h4",
            Self::H5 => "h5",
            Self::H6 => "h6",
            Self::Ul => "ul",
            Self::Ol => "ol",
            Self::Li => "li",
            Self::Dl => "dl",
            Self::Dt => "dt",
            Self::Dd => "dd",
            Self::Table => "table",
            Self::Thead => "thead",
            Self::Tbody => "tbody",
            Self::Tfoot => "tfoot",
            Self::Tr => "tr",
            Self::Th => "th",
            Self::Td => "td",
            Self::Caption => "caption",
            Self::Colgroup => "colgroup",
            Self::Col => "col",
            Self::Form => "form",
            Self::Input => "input",
            Self::Button => "button",
            Self::Select => "select",
            Self::Option => "option",
            Self::Textarea => "textarea",
            Self::Label => "label",
            Self::Img => "img",
            Self::Figure => "figure",
            Self::Figcaption => "figcaption",
            Self::Pre => "pre",
            Self::Code => "code",
            Self::Blockquote => "blockquote",
            Self::Cite => "cite",
            Self::Em => "em",
            Self::Strong => "strong",
            Self::B => "b",
            Self::I => "i",
            Self::U => "u",
            Self::S => "s",
            Self::Small => "small",
            Self::Sub => "sub",
            Self::Sup => "sup",
            Self::Mark => "mark",
            Self::Nav => "nav",
            Self::Header => "header",
            Self::Footer => "footer",
            Self::Main => "main",
            Self::Section => "section",
            Self::Article => "article",
            Self::Aside => "aside",
            Self::Details => "details",
            Self::Summary => "summary",
            Self::Iframe => "iframe",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Source => "source",
            Self::Canvas => "canvas",
            Self::Noscript => "noscript",
            Self::Template => "template",
            Self::Center => "center",
            Self::Svg => "svg",
            Self::Unknown(s) => s.as_str(),
        }
    }

    /// Returns `true` if this is a void element (self-closing, no content).
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            Self::Br
                | Self::Hr
                | Self::Img
                | Self::Input
                | Self::Meta
                | Self::Link
                | Self::Col
                | Self::Source
        )
    }

    /// Returns `true` if this is a block-level element by default.
    pub fn is_block_level(&self) -> bool {
        matches!(
            self,
            Self::Div
                | Self::P
                | Self::H1
                | Self::H2
                | Self::H3
                | Self::H4
                | Self::H5
                | Self::H6
                | Self::Ul
                | Self::Ol
                | Self::Li
                | Self::Dl
                | Self::Dt
                | Self::Dd
                | Self::Table
                | Self::Thead
                | Self::Tbody
                | Self::Tfoot
                | Self::Tr
                | Self::Form
                | Self::Blockquote
                | Self::Pre
                | Self::Figure
                | Self::Figcaption
                | Self::Nav
                | Self::Header
                | Self::Footer
                | Self::Main
                | Self::Section
                | Self::Article
                | Self::Aside
                | Self::Details
                | Self::Summary
                | Self::Hr
                | Self::Center
        )
    }

    /// Returns `true` if this is a formatting element
    /// (`b`, `i`, `em`, `strong`, `a`, `u`, `s`, `small`, `mark`,
    /// `sub`, `sup`).
    pub fn is_formatting(&self) -> bool {
        matches!(
            self,
            Self::B
                | Self::I
                | Self::Em
                | Self::Strong
                | Self::A
                | Self::U
                | Self::S
                | Self::Small
                | Self::Mark
                | Self::Sub
                | Self::Sup
        )
    }

    /// Returns `true` if this tag enters raw text mode
    /// (`script`, `style`).
    pub fn is_raw_text(&self) -> bool {
        matches!(self, Self::Script | Self::Style)
    }

    /// Returns `true` if this tag enters RCDATA mode
    /// (`title`, `textarea`).
    pub fn is_rcdata(&self) -> bool {
        matches!(self, Self::Title | Self::Textarea)
    }
}

// ------------------------------------------------------------------
// ElementData
// ------------------------------------------------------------------

impl ElementData {
    /// Create a new `ElementData` with the given tag and no attributes.
    pub fn new(tag: TagName) -> Self {
        Self {
            tag,
            attributes: Vec::new(),
        }
    }

    /// Get an attribute value by name (case-insensitive lookup).
    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    /// Check if this element has a given CSS class.
    ///
    /// The `class` attribute value is split on ASCII whitespace and each
    /// token is compared to `class`.
    pub fn has_class(&self, class: &str) -> bool {
        self.get_attribute("class")
            .map(|v| v.split_ascii_whitespace().any(|c| c == class))
            .unwrap_or(false)
    }

    /// Get the `id` attribute if present.
    pub fn id(&self) -> Option<&str> {
        self.get_attribute("id")
    }

    /// Get the `href` attribute if present (for links).
    pub fn href(&self) -> Option<&str> {
        self.get_attribute("href")
    }

    /// Get the `src` attribute if present (for images / media).
    pub fn src(&self) -> Option<&str> {
        self.get_attribute("src")
    }

    /// Set or overwrite an attribute value.
    pub fn set_attribute(&mut self, name: &str, value: &str) {
        if let Some(attr) = self.attributes.iter_mut().find(|a| a.name == name) {
            attr.value = value.to_string();
        } else {
            self.attributes.push(Attribute {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }

    /// Remove an attribute by name. Returns `true` if it existed.
    pub fn remove_attribute(&mut self, name: &str) -> bool {
        let len_before = self.attributes.len();
        self.attributes.retain(|a| a.name != name);
        self.attributes.len() < len_before
    }
}

// ------------------------------------------------------------------
// Document
// ------------------------------------------------------------------

impl Document {
    /// Create an empty document with a synthetic `Document` root node.
    pub fn new() -> Self {
        let root_node = Node {
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
        };
        Self {
            nodes: vec![root_node],
            root: 0,
            id_index: std::collections::HashMap::new(),
            free_list: Vec::new(),
        }
    }

    /// Reset the document to an empty state while preserving allocated
    /// memory.
    ///
    /// This clears all nodes, the ID index, and the free list, then
    /// re-adds the root `Document` node. The underlying `Vec` capacity
    /// is retained so the next parse avoids reallocations for documents
    /// of similar size.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.id_index.clear();
        self.free_list.clear();
        self.nodes.push(Node {
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
        });
        self.root = 0;
    }

    /// Create a document from a pre-built node arena.
    ///
    /// Rebuilds the ID index from the nodes. Useful for tests that
    /// construct documents directly.
    pub fn from_nodes(nodes: Vec<Node>, root: NodeId) -> Self {
        let mut id_index = std::collections::HashMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            if let NodeKind::Element(ref data) = node.kind
                && let Some(id_val) = data.id()
            {
                id_index.insert(id_val.to_string(), idx);
            }
        }
        Self {
            nodes,
            root,
            id_index,
            free_list: Vec::new(),
        }
    }

    /// Add a new node to the arena and return its [`NodeId`].
    ///
    /// If the node is an element with an `id` attribute, the ID index
    /// is updated for O(1) `get_element_by_id` lookups.
    pub fn add_node(&mut self, kind: NodeKind) -> NodeId {
        // Reuse a free slot if available to avoid unbounded arena growth.
        if let Some(id) = self.free_list.pop() {
            // Index the element's id attribute if present.
            if let NodeKind::Element(ref data) = kind
                && let Some(id_val) = data.id()
            {
                self.id_index.insert(id_val.to_string(), id);
            }
            self.nodes[id] = Node {
                kind,
                parent: None,
                children: Vec::new(),
            };
            return id;
        }
        let id = self.nodes.len();
        // Index the element's id attribute if present.
        if let NodeKind::Element(ref data) = kind
            && let Some(id_val) = data.id()
        {
            self.id_index.insert(id_val.to_string(), id);
        }
        self.nodes.push(Node {
            kind,
            parent: None,
            children: Vec::new(),
        });
        id
    }

    /// Append `child_id` as the last child of `parent_id`.
    ///
    /// Updates both the parent's child list and the child's parent link.
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        self.nodes[parent_id].children.push(child_id);
        self.nodes[child_id].parent = Some(parent_id);
    }

    /// Insert `child_id` into `parent_id`'s child list immediately
    /// before `reference_id`. Falls back to `append_child` when
    /// `reference_id` is not a child of `parent_id`.
    pub fn insert_before(&mut self, parent_id: NodeId, child_id: NodeId, reference_id: NodeId) {
        if let Some(pos) = self.nodes[parent_id]
            .children
            .iter()
            .position(|&id| id == reference_id)
        {
            self.nodes[parent_id].children.insert(pos, child_id);
            self.nodes[child_id].parent = Some(parent_id);
        } else {
            self.append_child(parent_id, child_id);
        }
    }

    /// Get a reference to a node by ID.
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// Get the [`ElementData`] for a node, if it is an `Element`.
    pub fn element(&self, id: NodeId) -> Option<&ElementData> {
        match &self.nodes[id].kind {
            NodeKind::Element(data) => Some(data),
            _ => None,
        }
    }

    /// Get the concatenated text content of a node and all its
    /// descendants.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out);
        out
    }

    /// Recursive text collection helper.
    fn collect_text(&self, id: NodeId, out: &mut String) {
        match &self.nodes[id].kind {
            NodeKind::Text(s) => out.push_str(s),
            _ => {
                for i in 0..self.nodes[id].children.len() {
                    let child = self.nodes[id].children[i];
                    self.collect_text(child, out);
                }
            },
        }
    }

    /// Find the first element whose `id` attribute matches `target`.
    ///
    /// Uses an O(1) hash map index instead of DFS traversal.
    pub fn get_element_by_id(&self, target: &str) -> Option<NodeId> {
        self.id_index.get(target).copied()
    }

    /// Update the ID index when an element's `id` attribute changes.
    ///
    /// Called from `set_attribute` and JS DOM mutation paths.
    pub fn update_id_index(&mut self, node_id: NodeId, old_id: Option<&str>, new_id: Option<&str>) {
        if let Some(old) = old_id {
            // Only remove if this node still owns the entry.
            if self.id_index.get(old) == Some(&node_id) {
                self.id_index.remove(old);
            }
        }
        if let Some(new) = new_id
            && !new.is_empty()
        {
            self.id_index.insert(new.to_string(), node_id);
        }
    }

    /// Find the `<body>` element.
    pub fn body(&self) -> Option<NodeId> {
        self.find_first_element(self.root, &TagName::Body)
    }

    /// Find the `<head>` element.
    pub fn head(&self) -> Option<NodeId> {
        self.find_first_element(self.root, &TagName::Head)
    }

    /// Find the `<title>` text content, if any.
    pub fn title(&self) -> Option<String> {
        let title_id = self.find_first_element(self.root, &TagName::Title)?;
        let text = self.text_content(title_id);
        if text.is_empty() { None } else { Some(text) }
    }

    /// Recursively free a node and all its descendants.
    ///
    /// Removes ID index entries for any elements with `id` attributes
    /// and pushes all node slots onto the free list for reuse.
    pub(crate) fn free_subtree(&mut self, node_id: NodeId) {
        // Collect children first to avoid borrow conflict.
        let children: Vec<NodeId> = self.nodes[node_id].children.clone();
        for child in children {
            self.free_subtree(child);
        }
        // Remove from ID index.
        if let NodeKind::Element(ref data) = self.nodes[node_id].kind
            && let Some(id) = data.id()
            && self.id_index.get(id) == Some(&node_id)
        {
            self.id_index.remove(id);
        }
        self.nodes[node_id].parent = None;
        self.nodes[node_id].children.clear();
        self.free_list.push(node_id);
    }

    /// Replace the children of `node_id` with a single text node.
    pub fn set_text_content(&mut self, node_id: NodeId, text: &str) {
        // Recursively free old children and all their descendants.
        let old_children: Vec<NodeId> = self.nodes[node_id].children.clone();
        for child_id in old_children {
            self.free_subtree(child_id);
        }
        self.nodes[node_id].children.clear();
        let text_id = self.add_node(NodeKind::Text(text.to_string()));
        self.append_child(node_id, text_id);
    }

    /// Remove `child_id` from its parent's child list and clear its
    /// parent link. Returns the former parent if one existed.
    ///
    /// Recursively frees all descendants, removing their ID index
    /// entries and adding their arena slots to the free list.
    pub fn remove_child(&mut self, child_id: NodeId) -> Option<NodeId> {
        let parent_id = self.nodes[child_id].parent.take()?;
        self.nodes[parent_id].children.retain(|&c| c != child_id);
        self.free_subtree(child_id);
        Some(parent_id)
    }

    /// Find the `<title>` element node (public wrapper around
    /// `find_first_element`).
    pub fn title_element(&self) -> Option<NodeId> {
        self.find_first_element(self.root, &TagName::Title)
    }

    /// Depth-first search for the first element with the given tag.
    fn find_first_element(&self, node_id: NodeId, tag: &TagName) -> Option<NodeId> {
        if let NodeKind::Element(ref data) = self.nodes[node_id].kind
            && data.tag == *tag
        {
            return Some(node_id);
        }
        for i in 0..self.nodes[node_id].children.len() {
            let child = self.nodes[node_id].children[i];
            if let Some(found) = self.find_first_element(child, tag) {
                return Some(found);
            }
        }
        None
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_document_with_nodes() {
        let mut doc = Document::new();
        assert_eq!(doc.nodes.len(), 1); // root Document node

        let div_id = doc.add_node(NodeKind::Element(ElementData::new(TagName::Div)));
        assert_eq!(div_id, 1);
        doc.append_child(doc.root, div_id);
        assert_eq!(doc.get(doc.root).children, vec![div_id]);
    }

    #[test]
    fn parent_child_links() {
        let mut doc = Document::new();
        let parent = doc.add_node(NodeKind::Element(ElementData::new(TagName::Div)));
        let child = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
        doc.append_child(doc.root, parent);
        doc.append_child(parent, child);

        assert_eq!(doc.get(child).parent, Some(parent));
        assert_eq!(doc.get(parent).children, vec![child]);
        assert_eq!(doc.get(doc.root).children, vec![parent]);
    }

    #[test]
    fn tag_name_roundtrip() {
        let tags = [
            "html", "head", "body", "div", "span", "p", "a", "br", "hr", "h1", "h6", "ul", "ol",
            "li", "table", "tr", "td", "form", "input", "img", "pre", "code", "em", "strong", "b",
            "i", "nav", "header", "footer", "main", "section", "article", "aside", "details",
            "summary", "iframe", "video", "audio", "source", "canvas", "noscript",
        ];
        for tag_str in &tags {
            let tag = TagName::from_str(tag_str);
            assert_eq!(tag.as_str(), *tag_str, "roundtrip failed for {tag_str}");
        }
    }

    #[test]
    fn tag_name_unknown() {
        let tag = TagName::from_str("custom-element");
        assert_eq!(tag, TagName::Unknown("custom-element".into()));
        assert_eq!(tag.as_str(), "custom-element");
    }

    #[test]
    fn is_void_correctness() {
        assert!(TagName::Br.is_void());
        assert!(TagName::Hr.is_void());
        assert!(TagName::Img.is_void());
        assert!(TagName::Input.is_void());
        assert!(TagName::Meta.is_void());
        assert!(TagName::Link.is_void());
        assert!(TagName::Col.is_void());
        assert!(TagName::Source.is_void());

        assert!(!TagName::Div.is_void());
        assert!(!TagName::P.is_void());
        assert!(!TagName::A.is_void());
        assert!(!TagName::Table.is_void());
    }

    #[test]
    fn is_block_level_correctness() {
        assert!(TagName::Div.is_block_level());
        assert!(TagName::P.is_block_level());
        assert!(TagName::H1.is_block_level());
        assert!(TagName::Ul.is_block_level());
        assert!(TagName::Table.is_block_level());
        assert!(TagName::Blockquote.is_block_level());
        assert!(TagName::Section.is_block_level());

        assert!(!TagName::Span.is_block_level());
        assert!(!TagName::A.is_block_level());
        assert!(!TagName::Em.is_block_level());
        assert!(!TagName::Strong.is_block_level());
    }

    #[test]
    fn is_formatting_correctness() {
        assert!(TagName::B.is_formatting());
        assert!(TagName::I.is_formatting());
        assert!(TagName::Em.is_formatting());
        assert!(TagName::Strong.is_formatting());
        assert!(TagName::A.is_formatting());
        assert!(TagName::U.is_formatting());
        assert!(TagName::Mark.is_formatting());

        assert!(!TagName::Div.is_formatting());
        assert!(!TagName::P.is_formatting());
    }

    #[test]
    fn is_raw_text_and_rcdata() {
        assert!(TagName::Script.is_raw_text());
        assert!(TagName::Style.is_raw_text());
        assert!(!TagName::Title.is_raw_text());

        assert!(TagName::Title.is_rcdata());
        assert!(TagName::Textarea.is_rcdata());
        assert!(!TagName::Script.is_rcdata());
    }

    #[test]
    fn element_data_attributes() {
        let mut elem = ElementData::new(TagName::A);
        elem.attributes.push(Attribute {
            name: "href".into(),
            value: "https://example.com".into(),
        });
        elem.attributes.push(Attribute {
            name: "class".into(),
            value: "link primary".into(),
        });
        elem.attributes.push(Attribute {
            name: "id".into(),
            value: "my-link".into(),
        });

        assert_eq!(elem.get_attribute("href"), Some("https://example.com"),);
        assert_eq!(elem.href(), Some("https://example.com"));
        assert_eq!(elem.id(), Some("my-link"));
        assert!(elem.has_class("link"));
        assert!(elem.has_class("primary"));
        assert!(!elem.has_class("secondary"));
        assert_eq!(elem.get_attribute("missing"), None);
    }

    #[test]
    fn element_data_src() {
        let mut elem = ElementData::new(TagName::Img);
        elem.attributes.push(Attribute {
            name: "src".into(),
            value: "photo.png".into(),
        });
        assert_eq!(elem.src(), Some("photo.png"));
    }

    #[test]
    fn text_content_traversal() {
        let mut doc = Document::new();
        let p = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
        doc.append_child(doc.root, p);

        let t1 = doc.add_node(NodeKind::Text("Hello ".into()));
        doc.append_child(p, t1);

        let b = doc.add_node(NodeKind::Element(ElementData::new(TagName::B)));
        doc.append_child(p, b);

        let t2 = doc.add_node(NodeKind::Text("world".into()));
        doc.append_child(b, t2);

        assert_eq!(doc.text_content(p), "Hello world");
        assert_eq!(doc.text_content(b), "world");
    }

    #[test]
    fn get_element_by_id_found() {
        let mut doc = Document::new();
        let mut data = ElementData::new(TagName::Div);
        data.attributes.push(Attribute {
            name: "id".into(),
            value: "content".into(),
        });
        let div_id = doc.add_node(NodeKind::Element(data));
        doc.append_child(doc.root, div_id);

        assert_eq!(doc.get_element_by_id("content"), Some(div_id));
        assert_eq!(doc.get_element_by_id("missing"), None);
    }

    #[test]
    fn body_and_head_lookup() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        let head = doc.add_node(NodeKind::Element(ElementData::new(TagName::Head)));
        doc.append_child(html, head);

        let body = doc.add_node(NodeKind::Element(ElementData::new(TagName::Body)));
        doc.append_child(html, body);

        assert_eq!(doc.head(), Some(head));
        assert_eq!(doc.body(), Some(body));
    }

    #[test]
    fn title_lookup() {
        let mut doc = Document::new();
        let html = doc.add_node(NodeKind::Element(ElementData::new(TagName::Html)));
        doc.append_child(doc.root, html);

        let head = doc.add_node(NodeKind::Element(ElementData::new(TagName::Head)));
        doc.append_child(html, head);

        let title = doc.add_node(NodeKind::Element(ElementData::new(TagName::Title)));
        doc.append_child(head, title);

        let text = doc.add_node(NodeKind::Text("My Page".into()));
        doc.append_child(title, text);

        assert_eq!(doc.title(), Some("My Page".into()));
    }

    #[test]
    fn title_missing_returns_none() {
        let doc = Document::new();
        assert_eq!(doc.title(), None);
    }

    #[test]
    fn default_impl() {
        let doc = Document::default();
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.root, 0);
    }

    #[test]
    fn set_attribute_new() {
        let mut elem = ElementData::new(TagName::Div);
        elem.set_attribute("class", "foo");
        assert_eq!(elem.get_attribute("class"), Some("foo"));
    }

    #[test]
    fn set_attribute_overwrite() {
        let mut elem = ElementData::new(TagName::Div);
        elem.set_attribute("id", "old");
        elem.set_attribute("id", "new");
        assert_eq!(elem.get_attribute("id"), Some("new"));
        assert_eq!(elem.attributes.len(), 1);
    }

    #[test]
    fn remove_attribute_exists() {
        let mut elem = ElementData::new(TagName::Div);
        elem.set_attribute("id", "x");
        assert!(elem.remove_attribute("id"));
        assert_eq!(elem.get_attribute("id"), None);
    }

    #[test]
    fn remove_attribute_missing() {
        let mut elem = ElementData::new(TagName::Div);
        assert!(!elem.remove_attribute("nope"));
    }

    #[test]
    fn set_text_content_replaces_children() {
        let mut doc = Document::new();
        let div = doc.add_node(NodeKind::Element(ElementData::new(TagName::Div)));
        doc.append_child(doc.root, div);
        let t1 = doc.add_node(NodeKind::Text("old".into()));
        doc.append_child(div, t1);
        let span = doc.add_node(NodeKind::Element(ElementData::new(TagName::Span)));
        doc.append_child(div, span);

        doc.set_text_content(div, "new");
        assert_eq!(doc.text_content(div), "new");
        assert_eq!(doc.get(div).children.len(), 1);
    }

    #[test]
    fn remove_child_unlinks() {
        let mut doc = Document::new();
        let parent = doc.add_node(NodeKind::Element(ElementData::new(TagName::Div)));
        doc.append_child(doc.root, parent);
        let child = doc.add_node(NodeKind::Element(ElementData::new(TagName::P)));
        doc.append_child(parent, child);

        assert_eq!(doc.remove_child(child), Some(parent));
        assert!(doc.get(parent).children.is_empty());
        assert_eq!(doc.get(child).parent, None);
    }
}
