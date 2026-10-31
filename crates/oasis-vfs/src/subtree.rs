//! [`SubtreeVfs`]: one directory of another VFS presented as a whole VFS.

use std::ops::DerefMut;

use oasis_types::error::{Result, VfsError};

use crate::{FilePermissions, RenamePlan, Vfs, VfsContext, VfsEntry, VfsMetadata, normalize_path};

/// A view of the directory `root` of an inner VFS as a VFS of its own (a
/// chroot): path `/x` of the view is `<root>/x` of the inner VFS.
///
/// Every path is normalized first ([`normalize_path`] rejects `..` above
/// `/`), so nothing outside `root` can be named, read or written. The view
/// root itself cannot be removed or renamed. Error messages come from the
/// inner VFS and so name inner paths.
///
/// `V` is anything that dereferences to a `Vfs`: `&mut dyn Vfs` for a
/// short-lived view (the FTP server wraps the shell VFS this way each
/// poll), or an owned `Box<dyn Vfs>`.
pub struct SubtreeVfs<V> {
    inner: V,
    /// Normalized inner path of the view root (`/` = the whole VFS).
    root: String,
}

impl<V> SubtreeVfs<V> {
    /// View the directory `root` of `inner`. `root` is normalized; it need
    /// not exist yet (operations then fail until it does).
    ///
    /// # Errors
    ///
    /// When `root` cannot be normalized (`..` above `/`, NUL byte).
    pub fn new(inner: V, root: &str) -> Result<Self> {
        let root = normalize_path(root)?.into_owned();
        Ok(Self { inner, root })
    }

    /// The inner path of the view root.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Give back the wrapped VFS.
    pub fn into_inner(self) -> V {
        self.inner
    }

    /// The inner path for view path `path`.
    fn map(&self, path: &str) -> Result<String> {
        let path = normalize_path(path)?;
        Ok(match (self.root.as_str(), path.as_ref()) {
            (_, "/") => self.root.clone(),
            ("/", p) => p.to_string(),
            (root, p) => format!("{root}{p}"),
        })
    }

    /// Like [`map`](Self::map), refusing the view root itself.
    fn map_not_root(&self, path: &str, what: &str) -> Result<String> {
        if normalize_path(path)? == "/" {
            return Err(VfsError::Other(format!("cannot {what} root")).into());
        }
        self.map(path)
    }
}

impl<V, T> Vfs for SubtreeVfs<V>
where
    V: DerefMut<Target = T>,
    T: Vfs + ?Sized,
{
    fn readdir(&self, path: &str) -> Result<Vec<VfsEntry>> {
        self.inner.readdir(&self.map(path)?)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.inner.read(&self.map(path)?)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let path = self.map(path)?;
        self.inner.write(&path, data)
    }

    fn stat(&self, path: &str) -> Result<VfsMetadata> {
        self.inner.stat(&self.map(path)?)
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        let path = self.map(path)?;
        self.inner.mkdir(&path)
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        let path = self.map_not_root(path, "remove")?;
        self.inner.remove(&path)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let from = self.map_not_root(from, "rename")?;
        let to = self.map_not_root(to, "rename onto")?;
        self.inner.rename(&from, &to)
    }

    fn exists(&self, path: &str) -> bool {
        self.map(path).is_ok_and(|p| self.inner.exists(&p))
    }

    fn is_symlink(&self, path: &str) -> bool {
        self.map(path).is_ok_and(|p| self.inner.is_symlink(&p))
    }

    fn get_permissions(&self, path: &str) -> Result<FilePermissions> {
        self.inner.get_permissions(&self.map(path)?)
    }

    fn set_permissions(&mut self, path: &str, perms: FilePermissions) -> Result<()> {
        let path = self.map(path)?;
        self.inner.set_permissions(&path, perms)
    }

    fn context(&self) -> VfsContext {
        self.inner.context()
    }

    fn set_context(&mut self, ctx: VfsContext) {
        self.inner.set_context(ctx);
    }

    fn vfs_type(&self) -> &'static str {
        "SubtreeVfs"
    }

    fn validate_rename(&self, from: &str, to: &str) -> Result<RenamePlan> {
        self.inner.validate_rename(&self.map(from)?, &self.map(to)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryVfs;

    fn jail() -> MemoryVfs {
        let mut vfs = MemoryVfs::new();
        vfs.mkdir("/home/user").unwrap();
        vfs.write("/home/user/a.txt", b"a").unwrap();
        vfs.mkdir("/system").unwrap();
        vfs.write("/system/secret", b"s").unwrap();
        vfs
    }

    #[test]
    fn paths_map_into_the_root() {
        let mut inner = jail();
        let mut view = SubtreeVfs::new(&mut inner as &mut dyn Vfs, "/home").unwrap();
        assert_eq!(view.read("/user/a.txt").unwrap(), b"a");
        let names: Vec<String> = view
            .readdir("/")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["user"]);
        view.write("/user/b.txt", b"b").unwrap();
        drop(view);
        assert_eq!(inner.read("/home/user/b.txt").unwrap(), b"b");
    }

    #[test]
    fn nothing_outside_the_root_is_reachable() {
        let mut inner = jail();
        let mut view = SubtreeVfs::new(&mut inner as &mut dyn Vfs, "/home/").unwrap();
        assert_eq!(view.root(), "/home");
        assert!(view.read("/../system/secret").is_err());
        assert!(view.read("/user/../../system/secret").is_err());
        assert!(!view.exists("/system/secret"));
        assert!(view.write("/../system/x", b"x").is_err());
        assert!(view.rename("/user/a.txt", "/../system/a.txt").is_err());
        assert!(view.remove("/").is_err());
        assert!(view.rename("/", "/moved").is_err());
        assert!(inner.exists("/home/user/a.txt"));
    }

    #[test]
    fn root_slash_is_the_whole_vfs() {
        let mut inner = jail();
        let view = SubtreeVfs::new(&mut inner as &mut dyn Vfs, "/").unwrap();
        assert_eq!(view.read("/system/secret").unwrap(), b"s");
    }

    #[test]
    fn bad_roots_are_rejected() {
        let mut inner = jail();
        assert!(SubtreeVfs::new(&mut inner as &mut dyn Vfs, "/../x").is_err());
    }
}
