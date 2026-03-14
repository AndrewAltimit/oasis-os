//! Clipboard backend trait and in-memory implementation.

/// Platform-agnostic clipboard trait.
///
/// Each backend can provide a native implementation (e.g. SDL3 system clipboard,
/// browser `navigator.clipboard`). Platforms without native clipboard (PSP, UE5)
/// use the provided [`InMemoryClipboard`].
pub trait ClipboardBackend {
    /// Copy text to the clipboard.
    fn copy(&mut self, text: &str);

    /// Paste text from the clipboard. Returns `None` if the clipboard is empty.
    fn paste(&self) -> Option<String>;

    /// Returns `true` if the clipboard contains text.
    fn has_content(&self) -> bool;

    /// Clear the clipboard contents.
    fn clear(&mut self);
}

/// In-memory clipboard implementation.
///
/// Stores clipboard content in process memory. Suitable for platforms without
/// native clipboard support (PSP, UE5, headless testing).
#[derive(Debug, Default, Clone)]
pub struct InMemoryClipboard {
    content: Option<String>,
}

impl InMemoryClipboard {
    /// Create a new empty in-memory clipboard.
    #[must_use]
    pub fn new() -> Self {
        Self { content: None }
    }
}

impl ClipboardBackend for InMemoryClipboard {
    fn copy(&mut self, text: &str) {
        self.content = Some(text.to_owned());
    }

    fn paste(&self) -> Option<String> {
        self.content.clone()
    }

    fn has_content(&self) -> bool {
        self.content.is_some()
    }

    fn clear(&mut self) {
        self.content = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_clipboard() {
        let clipboard = InMemoryClipboard::new();
        assert!(!clipboard.has_content());
        assert_eq!(clipboard.paste(), None);
    }

    #[test]
    fn test_copy_paste() {
        let mut clipboard = InMemoryClipboard::new();
        clipboard.copy("hello world");
        assert!(clipboard.has_content());
        assert_eq!(clipboard.paste(), Some("hello world".to_owned()));
    }

    #[test]
    fn test_overwrite() {
        let mut clipboard = InMemoryClipboard::new();
        clipboard.copy("first");
        clipboard.copy("second");
        assert_eq!(clipboard.paste(), Some("second".to_owned()));
    }

    #[test]
    fn test_clear() {
        let mut clipboard = InMemoryClipboard::new();
        clipboard.copy("data");
        assert!(clipboard.has_content());
        clipboard.clear();
        assert!(!clipboard.has_content());
        assert_eq!(clipboard.paste(), None);
    }

    #[test]
    fn test_empty_string() {
        let mut clipboard = InMemoryClipboard::new();
        clipboard.copy("");
        assert!(clipboard.has_content());
        assert_eq!(clipboard.paste(), Some(String::new()));
    }

    #[test]
    fn test_default() {
        let clipboard = InMemoryClipboard::default();
        assert!(!clipboard.has_content());
    }
}
