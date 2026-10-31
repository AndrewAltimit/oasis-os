//! [`WindowId`]: the cheap-to-clone window identifier.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Shared, reference-counted window identifier.
///
/// Wraps `Rc<str>` for cheap cloning at 60fps. Compares equal with
/// `&str`, `String`, and other `WindowId` values, so existing call
/// sites that do `id == "browser"` keep working.
#[derive(Clone, Eq)]
pub struct WindowId(Rc<str>);

impl WindowId {
    /// Create a new window id from any string-like value.
    pub fn new(s: impl Into<Rc<str>>) -> Self {
        Self(s.into())
    }

    /// Borrow the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for WindowId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for WindowId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &*self.0)
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Hash for WindowId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self.0).hash(state);
    }
}

impl PartialEq for WindowId {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl PartialEq<str> for WindowId {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for WindowId {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for WindowId {
    fn eq(&self, other: &String) -> bool {
        &*self.0 == other.as_str()
    }
}

impl PartialEq<WindowId> for str {
    fn eq(&self, other: &WindowId) -> bool {
        self == &*other.0
    }
}

impl PartialEq<WindowId> for &str {
    fn eq(&self, other: &WindowId) -> bool {
        *self == &*other.0
    }
}

impl PartialEq<WindowId> for String {
    fn eq(&self, other: &WindowId) -> bool {
        self.as_str() == &*other.0
    }
}

impl From<String> for WindowId {
    fn from(s: String) -> Self {
        Self(Rc::from(s))
    }
}

impl From<&str> for WindowId {
    fn from(s: &str) -> Self {
        Self(Rc::from(s))
    }
}
