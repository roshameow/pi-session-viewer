//! Explicit data-sharing broker, NOT an OS sandbox for Pi/bash or other commands.
//! Sources are read-only; only an explicitly requested regular file is copied.
//! Lists never resolve bookmarks or probe authorized source paths. Unix descriptor
//! walks reject symlinks at every component, including during rename races.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

#[cfg(target_os = "macos")]
#[path = "data_access_native.rs"]
mod native;

const PAGE: usize = 200;
const MAX_FILE: u64 = 100 * 1024 * 1024;
const MAX_MANIFEST: u64 = 4 * 1024 * 1024;
const MAX_ROWS: usize = 1000;
const MAX_OFFSET: usize = 1_000_000;
static OPERATIONS: Mutex<()> = Mutex::new(());
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizedDirectory {
    pub id: String,
    pub name: String,
    pub path: String,
    pub created_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedDataFile {
    pub id: String,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub source_path: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    name: String,
    relative_path: String,
    is_directory: bool,
    size: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntries {
    relative_path: String,
    entries: Vec<DirectoryEntry>,
    has_more: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    row: AuthorizedDirectory,
    bookmark: Vec<u8>,
    identity: DirectoryIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
}
#[cfg(any(target_os = "macos", test))]
struct SelectedDirectory {
    path: String,
    bookmark: Vec<u8>,
    identity: DirectoryIdentity,
    #[cfg(target_os = "macos")]
    active: std::sync::Arc<native::Scope>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    grants: Vec<Grant>,
    imports: Vec<ImportedDataFile>,
}

fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("Random ID unavailable: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn relative(path: &str, allow_empty: bool) -> Result<Vec<&str>> {
    if path.is_empty() && allow_empty {
        return Ok(vec![]);
    }
    if path.is_empty() || path.len() > 4096 || path.contains(['\\', '\0']) {
        return Err("Invalid relative path".into());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(':'))
    {
        return Err("Absolute paths and path traversal are forbidden".into());
    }
    Ok(parts)
}
fn selected_path(path: &str, home: &Path) -> Result<()> {
    let p = Path::new(path);
    if !p.is_absolute()
        || path.contains('\0')
        || p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("Invalid selected directory".into());
    }
    // macOS's writable-volume firmlink spelling can refer to the same broad
    // home/Library roots. Normalize this known alias lexically, never by probing.
    #[cfg(target_os = "macos")]
    let alternate;
    #[cfg(target_os = "macos")]
    let p = if let Ok(rest) = p.strip_prefix("/System/Volumes/Data") {
        alternate = Path::new("/").join(rest);
        alternate.as_path()
    } else {
        p
    };
    // Lexical rejection only: do NOT enumerate/probe these protected directories.
    if p == Path::new("/")
        || p == home
        || p == home.join("Library")
        || p == home.join("Library/Containers")
        || p == home.join("Library/Group Containers")
    {
        return Err("Choose a specific data subfolder, not a broad system/home directory".into());
    }
    Ok(())
}
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| "Home directory unavailable; cannot validate selection".into())
}
fn validate(m: &Manifest, base: &Path) -> Result<()> {
    if m.version != 1 || m.grants.len() > MAX_ROWS || m.imports.len() > MAX_ROWS {
        return Err("Unsupported or oversized broker manifest".into());
    }
    let home = home()?;
    let mut ids = std::collections::HashSet::new();
    for g in &m.grants {
        if !valid_id(&g.row.id)
            || !ids.insert(&g.row.id)
            || g.bookmark.is_empty()
            || g.bookmark.len() > 65536
            || g.row.name.len() > 1024
            || g.row.path.len() > 4096
        {
            return Err("Unsafe authorization manifest".into());
        }
        selected_path(&g.row.path, &home)?;
    }
    for r in &m.imports {
        if !valid_id(&r.id)
            || !ids.insert(&r.id)
            || relative(&r.name, false)?.len() != 1
            || r.size > MAX_FILE
            || r.source_path.len() > 8192
            || Path::new(&r.path) != base.join("imports").join(&r.id).join(&r.name)
        {
            return Err("Unsafe import manifest".into());
        }
    }
    Ok(())
}

#[cfg(unix)]
mod fs_safe {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    fn cs(s: &std::ffi::OsStr) -> Result<CString> {
        CString::new(s.as_bytes()).map_err(|_| "NUL path".into())
    }
    fn io_err() -> String {
        format!("Data unavailable: {}", std::io::Error::last_os_error())
    }
    pub fn open_at(dir: &File, name: &str, flags: i32, mode: u32) -> Result<File> {
        let name = cs(name.as_ref())?;
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode as libc::c_uint,
            )
        };
        if fd < 0 {
            Err(io_err())
        } else {
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    // Metadata only, without following links. A missing managed path is safe
    // to reconcile; other errors or replacement symlinks are NOT absence.
    fn exists_at(dir: &File, name: &str) -> Result<bool> {
        let name = cs(name.as_ref())?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err(format!("Data unavailable: {error}"))
        }
    }
    fn unlink_managed(dir: &File, name: &str, directory: bool) -> Result<()> {
        let name = cs(name.as_ref())?;
        if unsafe {
            libc::unlinkat(
                dir.as_raw_fd(),
                name.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } == 0
        {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // Never recursively remove extra files that a task/user may have saved.
        if error.raw_os_error() == Some(libc::ENOENT)
            || (directory && error.raw_os_error() == Some(libc::ENOTEMPTY))
        {
            Ok(())
        } else {
            Err(format!("Data unavailable: {error}"))
        }
    }
    fn mkdir(dir: &File, name: &str) -> Result<()> {
        let n = cs(name.as_ref())?;
        if unsafe { libc::mkdirat(dir.as_raw_fd(), n.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
        {
            return Err(io_err());
        }
        Ok(())
    }
    pub fn dir_at(dir: &File, name: &str, create: bool) -> Result<File> {
        if create {
            mkdir(dir, name)?;
        }
        open_at(dir, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)
    }
    fn search_flags() -> i32 {
        #[cfg(target_os = "macos")]
        {
            libc::O_SEARCH
        }
        #[cfg(target_os = "linux")]
        {
            libc::O_PATH
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            libc::O_RDONLY
        }
    }
    pub fn absolute(path: &Path, create: bool) -> Result<File> {
        if !path.is_absolute() {
            return Err("Expected absolute broker path".into());
        }
        let mut dir = File::open("/").map_err(|e| e.to_string())?;
        let components: Vec<_> = path.components().collect();
        for (index, c) in components.iter().enumerate() {
            match c {
                Component::RootDir => {}
                Component::Normal(n) => {
                    let n = n.to_str().ok_or("Non-UTF8 directory path")?;
                    if create {
                        mkdir(&dir, n)?;
                    }
                    // Search-only ancestor handles avoid requesting directory read
                    // access to broad protected ancestors of the chosen subtree.
                    let flags = if index + 1 == components.len() {
                        libc::O_RDONLY
                    } else {
                        search_flags()
                    };
                    dir = open_at(&dir, n, flags | libc::O_DIRECTORY, 0)?;
                }
                _ => return Err("Invalid directory path".into()),
            }
        }
        Ok(dir)
    }
    pub(super) fn identity(f: &File) -> Result<DirectoryIdentity> {
        let metadata = f.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Authorized root is not a directory".into());
        }
        Ok(DirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub(super) fn require_identity(root: &File, expected: &DirectoryIdentity) -> Result<()> {
        if identity(root)? != *expected {
            return Err("Authorized root was replaced; remove it and choose again".into());
        }
        Ok(())
    }
    pub fn private(f: &File, directory: bool) -> Result<()> {
        let m = f.metadata().map_err(|e| e.to_string())?;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || (directory && !m.is_dir())
            || (!directory && (!m.is_file() || m.nlink() != 1))
        {
            return Err("Broker storage is not private, regular, and owned by this user".into());
        }
        Ok(())
    }
    pub fn unlink(dir: &File, name: &str, directory: bool) -> Result<()> {
        let n = cs(name.as_ref())?;
        if unsafe {
            libc::unlinkat(
                dir.as_raw_fd(),
                n.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(io_err());
        }
        Ok(())
    }
    fn rename(dir: &File, from: &str, to: &str) -> Result<()> {
        let a = cs(from.as_ref())?;
        let b = cs(to.as_ref())?;
        if unsafe { libc::renameat(dir.as_raw_fd(), a.as_ptr(), dir.as_raw_fd(), b.as_ptr()) } != 0
        {
            return Err(io_err());
        }
        Ok(())
    }
    pub struct Store {
        pub base: PathBuf,
        pub dir: File,
        pub manifest: Manifest,
    }
    impl Store {
        pub fn open(base: &Path) -> Result<Self> {
            let root = absolute(base, true)?;
            let dir = dir_at(&root, "data-access", true)?;
            private(&dir, true)?;
            // flock the stable directory inode, not a manifest replaced by rename.
            if unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(io_err());
            }
            let manifest =
                match open_at(&dir, "manifest.json", libc::O_RDONLY | libc::O_NONBLOCK, 0) {
                    Ok(f) => {
                        private(&f, false)?;
                        if f.metadata().map_err(|e| e.to_string())?.len() > MAX_MANIFEST {
                            return Err("Oversized broker manifest".into());
                        }
                        let mut bytes = Vec::new();
                        f.take(MAX_MANIFEST + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|e| e.to_string())?;
                        if bytes.len() as u64 > MAX_MANIFEST {
                            return Err("Oversized broker manifest".into());
                        }
                        serde_json::from_slice(&bytes)
                            .map_err(|e| format!("Invalid broker manifest: {e}"))?
                    }
                    Err(e) => {
                        // Only a genuinely absent file is an empty manifest. Never reset corruption.
                        let n = cs("manifest.json".as_ref())?;
                        let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
                        if unsafe {
                            libc::fstatat(
                                dir.as_raw_fd(),
                                n.as_ptr(),
                                st.as_mut_ptr(),
                                libc::AT_SYMLINK_NOFOLLOW,
                            )
                        } == -1
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
                        {
                            Manifest {
                                version: 1,
                                ..Default::default()
                            }
                        } else {
                            return Err(e);
                        }
                    }
                };
            validate(&manifest, base)?;
            Ok(Self {
                base: base.into(),
                dir,
                manifest,
            })
        }
        pub fn save(&self) -> Result<()> {
            self.save_tracking(&mut false)
        }
        fn save_tracking(&self, committed: &mut bool) -> Result<()> {
            *committed = false;
            validate(&self.manifest, &self.base)?;
            let bytes = serde_json::to_vec(&self.manifest).map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_MANIFEST {
                return Err("Broker manifest full".into());
            }
            let temp = format!("manifest-{}.tmp", new_id()?);
            let result = (|| {
                let mut file = open_at(
                    &self.dir,
                    &temp,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?;
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                rename(&self.dir, &temp, "manifest.json")?;
                *committed = true;
                self.dir.sync_all().map_err(|e| {
                    format!("Broker change committed, but durability sync failed: {e}")
                })
            })();
            if result.is_err() {
                let _ = unlink(&self.dir, &temp, false);
            }
            result
        }
        pub fn imports(&self) -> Result<File> {
            let base = absolute(&self.base, false)?;
            let dir = dir_at(&base, "imports", true)?;
            private(&dir, true)?;
            Ok(dir)
        }
        pub fn revoke(&mut self, id: &str) -> Result<()> {
            if !valid_id(id) {
                return Err("Invalid authorization ID".into());
            }
            self.manifest.grants.retain(|g| g.row.id != id);
            self.save()
        }
        pub fn copy(
            &mut self,
            root: &File,
            source_root: &str,
            path: &str,
        ) -> Result<ImportedDataFile> {
            let parts = relative(path, false)?;
            let name = *parts.last().ok_or("Choose one file")?;
            let mut src = descendant(root, &parts, false)?;
            let meta = src.metadata().map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err("Only regular files can be imported".into());
            }
            if meta.len() > MAX_FILE {
                return Err("File exceeds 100 MiB limit".into());
            }
            if self.manifest.imports.len() >= MAX_ROWS {
                return Err("Import manifest full".into());
            }
            let id = new_id()?;
            let imports = self.imports()?;
            let n = cs(id.as_ref())?;
            if unsafe { libc::mkdirat(imports.as_raw_fd(), n.as_ptr(), 0o700) } != 0 {
                return Err(io_err());
            }
            let dir = match dir_at(&imports, &id, false) {
                Ok(dir) => dir,
                Err(e) => {
                    let _ = unlink(&imports, &id, true);
                    return Err(e);
                }
            };
            let mut created = false;
            let mut committed = false;
            let result = (|| {
                private(&dir, true)?;
                let mut dest = open_at(
                    &dir,
                    name,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                    0o600,
                )?;
                created = true;
                let size = std::io::copy(&mut (&mut src).take(MAX_FILE + 1), &mut dest)
                    .map_err(|e| e.to_string())?;
                if size > MAX_FILE {
                    return Err("File grew beyond 100 MiB limit".into());
                }
                dest.sync_all().map_err(|e| e.to_string())?;
                dir.sync_all().map_err(|e| e.to_string())?;
                imports.sync_all().map_err(|e| e.to_string())?;
                let row = ImportedDataFile {
                    id: id.clone(),
                    path: self
                        .base
                        .join("imports")
                        .join(&id)
                        .join(name)
                        .to_str()
                        .ok_or("Non-UTF8 import path")?
                        .into(),
                    name: name.into(),
                    size,
                    source_path: format!("{}/{}", source_root.trim_end_matches('/'), path),
                };
                self.manifest.imports.push(row.clone());
                if let Err(e) = self.save_tracking(&mut committed) {
                    if !committed {
                        self.manifest.imports.pop();
                    }
                    return Err(e);
                }
                Ok(row)
            })();
            if result.is_err() && !committed {
                // Never delete a committed import, nor a file we didn't create.
                if created {
                    let _ = unlink(&dir, name, false);
                }
                let _ = unlink(&imports, &id, true);
            }
            result
        }
        pub fn remove_import(&mut self, id: &str) -> Result<()> {
            if !valid_id(id) {
                return Err("Invalid import ID".into());
            }
            let Some(row) = self.manifest.imports.iter().find(|r| r.id == id).cloned() else {
                return Ok(());
            };
            let imports = self.imports()?;
            if exists_at(&imports, id)? {
                let dir = dir_at(&imports, id, false)?;
                private(&dir, true)?;
                // Only the managed file, never recursive delete or stored paths.
                if exists_at(&dir, &row.name)? {
                    let f = open_at(&dir, &row.name, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
                    private(&f, false)?;
                    unlink_managed(&dir, &row.name, false)?;
                }
                unlink_managed(&imports, id, true)?;
            }
            // Missing copy/dir also clears its stale receipt. Restore memory on
            // persistence failure so retries work both here and after reload.
            let index = self
                .manifest
                .imports
                .iter()
                .position(|r| r.id == id)
                .unwrap();
            let removed = self.manifest.imports.remove(index);
            if let Err(error) = self.save() {
                self.manifest.imports.insert(index, removed);
                return Err(error);
            }
            Ok(())
        }
    }
    pub fn descendant(root: &File, parts: &[&str], directory: bool) -> Result<File> {
        let mut f = root.try_clone().map_err(|e| e.to_string())?;
        for (i, p) in parts.iter().enumerate() {
            let is_dir = i + 1 < parts.len() || directory;
            let flags = if i + 1 < parts.len() {
                search_flags()
            } else {
                libc::O_RDONLY | libc::O_NONBLOCK
            };
            f = open_at(&f, p, flags | if is_dir { libc::O_DIRECTORY } else { 0 }, 0)?;
        }
        Ok(f)
    }
    struct Dir(*mut libc::DIR);
    impl Drop for Dir {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    pub fn entries(root: &File, path: &str, offset: usize) -> Result<DirectoryEntries> {
        if offset > MAX_OFFSET {
            return Err("Listing offset exceeds limit".into());
        }
        let parts = relative(path, true)?;
        let dir = descendant(root, &parts, true)?;
        // A fresh descriptor has an independent stream offset (dup would share it).
        let stream = open_at(&dir, ".", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        use std::os::fd::IntoRawFd;
        let fd = stream.into_raw_fd();
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(io_err());
        }
        let stream = Dir(raw);
        let mut rows = Vec::with_capacity(PAGE);
        let mut skipped = 0;
        loop {
            // readdir NULL can mean error; clear/check errno, platform-specific.
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                #[cfg(target_os = "macos")]
                let errno = unsafe { *libc::__error() };
                #[cfg(target_os = "linux")]
                let errno = unsafe { *libc::__errno_location() };
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                let errno = 0;
                if errno != 0 {
                    return Err(io_err());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            let Ok(name_str) = name.to_str() else {
                continue;
            };
            if name_str == "." || name_str == ".." || relative(name_str, false).is_err() {
                continue;
            }
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(io_err());
            }
            let st = unsafe { st.assume_init() };
            let kind = st.st_mode & libc::S_IFMT;
            if kind != libc::S_IFDIR && kind != libc::S_IFREG {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            if rows.len() == PAGE {
                return Ok(DirectoryEntries {
                    relative_path: path.into(),
                    entries: rows,
                    has_more: true,
                });
            }
            rows.push(DirectoryEntry {
                name: name_str.into(),
                relative_path: if path.is_empty() {
                    name_str.into()
                } else {
                    format!("{path}/{name_str}")
                },
                is_directory: kind == libc::S_IFDIR,
                size: if kind == libc::S_IFREG {
                    st.st_size.max(0) as u64
                } else {
                    0
                },
            });
        }
        Ok(DirectoryEntries {
            relative_path: path.into(),
            entries: rows,
            has_more: false,
        })
    }
}

fn app_base(app: &tauri::AppHandle) -> Result<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("App data directory unavailable: {e}"))
}
#[cfg(unix)]
fn with_store<T>(
    app: &tauri::AppHandle,
    f: impl FnOnce(&mut fs_safe::Store) -> Result<T>,
) -> Result<T> {
    let _lock = OPERATIONS.lock().map_err(|_| "Broker lock poisoned")?;
    let mut store = fs_safe::Store::open(&app_base(app)?)?;
    f(&mut store)
}
#[cfg(not(unix))]
fn unsupported<T>() -> Result<T> {
    Err("Data-sharing broker is unsupported on this platform".into())
}

#[tauri::command]
pub async fn list_authorized_directories(
    app: tauri::AppHandle,
) -> Result<Vec<AuthorizedDirectory>> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            with_store(&app, |s| {
                Ok(s.manifest.grants.iter().map(|g| g.row.clone()).collect())
            })
        }
        #[cfg(not(unix))]
        {
            let _ = app;
            unsupported()
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
// Keep cancellation outside storage initialization and mutation. The callback
// boundary also permits a regression test without presenting a live picker.
#[cfg(any(target_os = "macos", test))]
fn finish_selection(
    selection: Option<SelectedDirectory>,
    save: impl FnOnce(SelectedDirectory) -> Result<Option<AuthorizedDirectory>>,
) -> Result<Option<AuthorizedDirectory>> {
    match selection {
        None => Ok(None),
        Some(selection) => save(selection),
    }
}

#[cfg(any(target_os = "macos", test))]
struct PickerGuard;
#[cfg(any(target_os = "macos", test))]
static PICKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(any(target_os = "macos", test))]
impl PickerGuard {
    fn begin() -> Result<Self> {
        PICKING
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| "A system folder chooser is already open")?;
        Ok(Self)
    }
}
#[cfg(any(target_os = "macos", test))]
impl Drop for PickerGuard {
    fn drop(&mut self) {
        PICKING.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[tauri::command]
pub async fn choose_authorized_directory(
    app: tauri::AppHandle,
) -> Result<Option<AuthorizedDirectory>> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Native directory authorization is supported only on macOS".into())
    }
    #[cfg(target_os = "macos")]
    {
        // Guard survives the modal run loop, panel remount and persistence.
        let _picker = PickerGuard::begin()?;
        let (tx, rx) = tauri::async_runtime::channel(1);
        app.run_on_main_thread(move || {
            let _ = tx.try_send(native::choose());
        })
        .map_err(|e| e.to_string())?;
        let mut rx = rx;
        let selection = rx.recv().await.ok_or("Chooser did not respond")??;
        tauri::async_runtime::spawn_blocking(move || {
            finish_selection(
                selection,
                |SelectedDirectory {
                     path,
                     bookmark,
                     identity,
                     active,
                 }| {
                    with_store(&app, |s| {
                        selected_path(&path, &home()?)?;
                        let existing = s.manifest.grants.iter().position(|g| g.row.path == path);
                        if existing.is_none() && s.manifest.grants.len() >= MAX_ROWS {
                            return Err("Authorization manifest full".into());
                        }
                        let row = AuthorizedDirectory {
                            id: match existing {
                                Some(i) => s.manifest.grants[i].row.id.clone(),
                                None => new_id()?,
                            },
                            name: Path::new(&path)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .ok_or("Invalid directory name")?
                                .into(),
                            path,
                            created_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_err(|e| e.to_string())?
                                .as_millis() as u64,
                        };
                        let grant = Grant {
                            row: row.clone(),
                            bookmark,
                            identity,
                        };
                        match existing {
                            Some(i) => s.manifest.grants[i] = grant,
                            None => s.manifest.grants.push(grant),
                        }
                        s.save()?;
                        // Only successful explicit selection/persistence can seed
                        // process-local picker authority. No persisted path bypass.
                        native::remember(&row.id, active)?;
                        Ok(Some(row))
                    })
                },
            )
        })
        .await
        .map_err(|e| e.to_string())?
    }
}
#[tauri::command]
pub async fn remove_authorized_directory(app: tauri::AppHandle, id: String) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            with_store(&app, |s| {
                s.revoke(&id)?;
                #[cfg(target_os = "macos")]
                native::forget(&id);
                Ok(())
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (app, id);
            unsupported()
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(target_os = "macos")]
fn scoped<T>(
    s: &mut fs_safe::Store,
    id: &str,
    f: impl FnOnce(&mut fs_safe::Store, &std::fs::File, &str) -> Result<T>,
) -> Result<T> {
    let idx = s
        .manifest
        .grants
        .iter()
        .position(|g| g.row.id == id)
        .ok_or("Directory authorization unavailable or revoked")?;
    let scope = native::resolve(
        id,
        &s.manifest.grants[idx].bookmark,
        &s.manifest.grants[idx].row.path,
        &s.manifest.grants[idx].identity,
    )?;
    selected_path(&scope.path, &home()?)?;
    if scope.path != s.manifest.grants[idx].row.path {
        return Err("Authorized folder moved; remove it and choose again".into());
    }
    // Native resolution retains the no-follow root descriptor with the scope;
    // operations never reopen the pathname after the identity check.
    let root = scope.root();
    if let Some(bookmark) = &scope.refreshed {
        s.manifest.grants[idx].bookmark = bookmark.clone();
        s.save()?;
    }
    f(s, root, &scope.path)
    // scope Drop balances only successful startAccess calls, even on error.
}
#[tauri::command]
pub async fn authorized_directory_entries(
    app: tauri::AppHandle,
    id: String,
    relative_path: String,
    offset: Option<usize>,
) -> Result<DirectoryEntries> {
    tauri::async_runtime::spawn_blocking(move || {
        relative(&relative_path, true)?;
        if offset.unwrap_or(0) > MAX_OFFSET {
            return Err("Listing offset exceeds limit".into());
        }
        #[cfg(target_os = "macos")]
        {
            with_store(&app, |s| {
                scoped(s, &id, |_, root, _| {
                    fs_safe::entries(root, &relative_path, offset.unwrap_or(0))
                })
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app, id);
            Err("Directory bookmark access is supported only on macOS".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn import_authorized_file(
    app: tauri::AppHandle,
    id: String,
    relative_path: String,
) -> Result<ImportedDataFile> {
    tauri::async_runtime::spawn_blocking(move || {
        relative(&relative_path, false)?;
        #[cfg(target_os = "macos")]
        {
            with_store(&app, |s| {
                scoped(s, &id, |s, root, source| {
                    s.copy(root, source, &relative_path)
                })
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app, id);
            Err("Directory bookmark access is supported only on macOS".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn list_imported_data_files(app: tauri::AppHandle) -> Result<Vec<ImportedDataFile>> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            with_store(&app, |s| Ok(s.manifest.imports.clone()))
        }
        #[cfg(not(unix))]
        {
            let _ = app;
            unsupported()
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn remove_imported_data_file(app: tauri::AppHandle, id: String) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(unix)]
        {
            with_store(&app, |s| s.remove_import(&id))
        }
        #[cfg(not(unix))]
        {
            let _ = (app, id);
            unsupported()
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod data_access_tests {
    use super::*;
    #[test]
    fn chooser_guard_blocks_reentry_and_releases_after_drop() {
        let first = PickerGuard::begin().unwrap();
        assert!(PickerGuard::begin().is_err());
        drop(first);
        assert!(PickerGuard::begin().is_ok());
    }
    #[test]
    fn lexical_traversal_and_broad_roots() {
        for p in [
            "/absolute",
            "..",
            "a/../b",
            "./a",
            "a//b",
            "a/",
            "C:/file",
            "a\\b",
            "a\0b",
            "",
        ] {
            assert!(relative(p, false).is_err(), "accepted {p:?}");
        }
        assert!(relative("", true).unwrap().is_empty());
        assert_eq!(
            relative("specific/attachment.txt", false).unwrap(),
            ["specific", "attachment.txt"]
        );
        let h = Path::new("/Users/mock-user");
        for p in [
            "/",
            "/Users/mock-user",
            "/Users/mock-user/Library",
            "/Users/mock-user/Library/Containers",
            "/Users/mock-user/Library/Group Containers",
        ] {
            assert!(selected_path(p, h).is_err());
        }
        assert!(selected_path(
            "/Users/mock-user/Library/Containers/com.tencent.xinWeChat/Data/Attachments",
            h
        )
        .is_ok());
        assert!(selected_path("/Users/mock-user/Library/../Library", h).is_err());
        #[cfg(target_os = "macos")]
        for p in [
            "/System/Volumes/Data",
            "/System/Volumes/Data/Users/mock-user",
            "/System/Volumes/Data/Users/mock-user/Library/Containers",
        ] {
            assert!(selected_path(p, h).is_err());
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use crate::data_access::fs_safe::*;
        use std::fs;
        use std::io::Write;
        use std::os::unix::fs::{symlink, PermissionsExt};
        struct Fixture(PathBuf);
        impl Fixture {
            fn new() -> Self {
                // macOS /var is a symlink. Resolve ONLY our test temp base.
                let base = std::env::temp_dir()
                    .canonicalize()
                    .unwrap()
                    .join(format!("viewer-broker-test-{}", new_id().unwrap()));
                fs::create_dir(&base).unwrap();
                fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
                Self(base)
            }
            fn source(&self) -> std::fs::File {
                let source = self.0.join("source");
                fs::create_dir(&source).unwrap();
                absolute(&source, false).unwrap()
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        fn grant(path: &Path) -> Grant {
            Grant {
                row: AuthorizedDirectory {
                    id: new_id().unwrap(),
                    name: "fixture".into(),
                    path: path.to_str().unwrap().into(),
                    created_at: 0,
                },
                bookmark: vec![1, 2, 3],
                identity: DirectoryIdentity {
                    device: 1,
                    inode: 1,
                },
            }
        }
        #[test]
        fn symlink_escapes_and_nonregular_files_are_blocked() {
            let f = Fixture::new();
            let root = f.source();
            fs::create_dir(f.0.join("outside")).unwrap();
            fs::write(f.0.join("outside/secret"), b"fixture-only").unwrap();
            symlink(f.0.join("outside"), f.0.join("source/link-dir")).unwrap();
            symlink(f.0.join("outside/secret"), f.0.join("source/link-file")).unwrap();
            fs::create_dir(f.0.join("source/dir")).unwrap();
            let mut s = Store::open(&f.0).unwrap();
            for p in ["link-dir/secret", "link-file", "dir", "../outside/secret"] {
                assert!(s.copy(&root, "/mock/source", p).is_err(), "accepted {p}");
            }
            assert!(entries(&root, "link-dir", 0).is_err());
            let page = entries(&root, "", 0).unwrap();
            assert_eq!(page.entries.len(), 1); // symlinks are omitted, never followed.
            assert!(page.entries[0].is_directory);
            let fifo = std::ffi::CString::new(f.0.join("source/fifo").to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert!(s.copy(&root, "/mock/source", "fifo").is_err());
            assert_eq!(entries(&root, "", 0).unwrap().entries.len(), 1);
            symlink(f.0.join("source"), f.0.join("root-link")).unwrap();
            assert!(absolute(&f.0.join("root-link"), false).is_err());
            assert!(s.manifest.imports.is_empty());
        }
        #[test]
        fn pages_and_file_size_are_bounded() {
            let f = Fixture::new();
            let root = f.source();
            for i in 0..201 {
                fs::write(f.0.join(format!("source/{i:03}")), b"x").unwrap();
            }
            let a = entries(&root, "", 0).unwrap();
            assert_eq!(a.entries.len(), PAGE);
            assert!(a.has_more);
            let b = entries(&root, "", PAGE).unwrap();
            assert_eq!(b.entries.len(), 1);
            assert!(!b.has_more);
            assert!(entries(&root, "", MAX_OFFSET + 1).is_err());
            let huge = fs::File::create(f.0.join("source/huge")).unwrap();
            huge.set_len(MAX_FILE + 1).unwrap(); // Sparse fixture, no 100 MiB allocation.
            let mut s = Store::open(&f.0).unwrap();
            assert!(s
                .copy(&root, "/mock/source", "huge")
                .unwrap_err()
                .contains("100 MiB"));
            assert!(s.manifest.imports.is_empty());
            assert!(!f.0.join("imports").exists());
        }
        #[test]
        fn single_file_copy_private_storage_revocation_and_removal() {
            let f = Fixture::new();
            let root = f.source();
            fs::write(f.0.join("source/one.txt"), b"selected only").unwrap();
            fs::write(f.0.join("source/two.txt"), b"not selected").unwrap();
            let mut s = Store::open(&f.0).unwrap();
            let g = grant(&f.0.join("source"));
            let id = g.row.id.clone();
            s.manifest.grants.push(g);
            s.save().unwrap();
            let r = s
                .copy(&root, f.0.join("source").to_str().unwrap(), "one.txt")
                .unwrap();
            let second = s.copy(&root, "/mock/source", "one.txt").unwrap();
            assert_ne!(r.id, second.id); // Never overwrite an existing import.
            assert_eq!(fs::read(&r.path).unwrap(), b"selected only");
            assert_eq!(r.size, 13);
            assert_eq!(
                fs::read_dir(Path::new(&r.path).parent().unwrap())
                    .unwrap()
                    .count(),
                1
            );
            assert_eq!(
                fs::metadata(&r.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(f.0.join("data-access/manifest.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            s.revoke(&id).unwrap();
            assert!(s.manifest.grants.is_empty());
            assert!(Path::new(&r.path).exists()); // Revoking never removes copies.
            assert_eq!(
                fs::read(f.0.join("source/one.txt")).unwrap(),
                b"selected only"
            );
            s.remove_import(&r.id).unwrap();
            assert!(!Path::new(&r.path).exists());
            assert!(s.remove_import("../source").is_err());
            drop(s);
            let s = Store::open(&f.0).unwrap();
            assert!(s.manifest.grants.is_empty());
            assert_eq!(s.manifest.imports.len(), 1);
        }
        #[test]
        fn missing_and_interrupted_import_deletion_is_retryable() {
            let f = Fixture::new();
            let root = f.source();
            fs::write(f.0.join("source/one.txt"), b"fixture only").unwrap();
            let mut store = Store::open(&f.0).unwrap();
            let missing = store.copy(&root, "/mock/source", "one.txt").unwrap();
            fs::remove_file(&missing.path).unwrap();
            store.remove_import(&missing.id).unwrap();
            assert!(store.manifest.imports.is_empty());
            let gone = store.copy(&root, "/mock/source", "one.txt").unwrap();
            fs::remove_file(&gone.path).unwrap();
            fs::remove_dir(Path::new(&gone.path).parent().unwrap()).unwrap();
            store.remove_import(&gone.id).unwrap();
            let partial = store.copy(&root, "/mock/source", "one.txt").unwrap();
            store.manifest.version = 999; // Fail validation AFTER filesystem deletion.
            assert!(store.remove_import(&partial.id).is_err());
            assert!(!Path::new(&partial.path).exists());
            assert_eq!(store.manifest.imports.len(), 1);
            drop(store);
            let mut store = Store::open(&f.0).unwrap(); // Original receipt remains on disk.
            assert_eq!(store.manifest.imports.len(), 1);
            store.remove_import(&partial.id).unwrap();
            store.remove_import(&partial.id).unwrap(); // Idempotent retry.
            assert!(store.manifest.imports.is_empty());
            drop(store);
            assert!(Store::open(&f.0).unwrap().manifest.imports.is_empty());
            assert_eq!(
                fs::read(f.0.join("source/one.txt")).unwrap(),
                b"fixture only"
            );
        }
        #[test]
        fn deletion_preserves_unmanaged_files_and_refuses_replacement_symlinks() {
            let f = Fixture::new();
            let root = f.source();
            fs::write(f.0.join("source/one.txt"), b"source stays").unwrap();
            let mut store = Store::open(&f.0).unwrap();
            let copied = store.copy(&root, "/mock/source", "one.txt").unwrap();
            let extra = Path::new(&copied.path)
                .parent()
                .unwrap()
                .join("user-result.txt");
            fs::write(&extra, b"keep this").unwrap();
            store.remove_import(&copied.id).unwrap();
            assert!(store.manifest.imports.is_empty());
            assert_eq!(fs::read(&extra).unwrap(), b"keep this");
            let replaced = store.copy(&root, "/mock/source", "one.txt").unwrap();
            fs::remove_file(&replaced.path).unwrap();
            symlink(f.0.join("source/one.txt"), &replaced.path).unwrap();
            assert!(store.remove_import(&replaced.id).is_err());
            assert_eq!(store.manifest.imports.len(), 1);
            assert_eq!(
                fs::read(f.0.join("source/one.txt")).unwrap(),
                b"source stays"
            );
            fs::remove_file(&replaced.path).unwrap();
            store.remove_import(&replaced.id).unwrap();
            assert!(store.manifest.imports.is_empty());
        }
        #[test]
        fn manifest_cannot_reset_corruption_or_redirect_operations() {
            let f = Fixture::new();
            let s = Store::open(&f.0).unwrap();
            s.save().unwrap();
            drop(s);
            let manifest = f.0.join("data-access/manifest.json");
            let sentinel = f.0.join("outside-sentinel");
            fs::write(&sentinel, b"unchanged").unwrap();
            fs::remove_file(&manifest).unwrap();
            symlink(&sentinel, &manifest).unwrap();
            assert!(Store::open(&f.0).is_err());
            assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
            fs::remove_file(&manifest).unwrap();
            fs::write(&manifest, b"not json").unwrap();
            fs::set_permissions(&manifest, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(Store::open(&f.0).is_err());
            let mut bad = Manifest {
                version: 1,
                ..Default::default()
            };
            bad.imports.push(ImportedDataFile {
                id: new_id().unwrap(),
                name: "outside-sentinel".into(),
                path: sentinel.to_str().unwrap().into(),
                source_path: "/mock/source".into(),
                size: 9,
            });
            fs::write(&manifest, serde_json::to_vec(&bad).unwrap()).unwrap();
            assert!(Store::open(&f.0).is_err());
            bad.imports[0].id = "../outside".into();
            assert!(validate(&bad, &f.0).is_err());
            bad.imports.clear();
            bad.grants.push(grant(Path::new("/")));
            assert!(validate(&bad, &f.0).is_err());
            let file = fs::File::create(&manifest).unwrap();
            file.set_len(MAX_MANIFEST + 1).unwrap();
            assert!(Store::open(&f.0).is_err());
            file.set_len(0).unwrap();
            (&file)
                .write_all(br#"{"version":1,"grants":[],"imports":[],"unknown":1}"#)
                .unwrap();
            assert!(Store::open(&f.0).is_err());
            fs::write(&manifest, br#"{"version":1,"grants":[],"imports":[]}"#).unwrap();
            fs::set_permissions(&manifest, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(Store::open(&f.0).is_err());
        }
        #[test]
        fn root_identity_and_descriptor_survive_path_replacement_safely() {
            let f = Fixture::new();
            let root = f.source();
            fs::write(f.0.join("source/original.txt"), b"original selected root").unwrap();
            let approved = identity(&root).unwrap();
            require_identity(&root, &approved).unwrap();
            fs::rename(f.0.join("source"), f.0.join("moved-source")).unwrap();
            fs::create_dir(f.0.join("source")).unwrap();
            fs::write(
                f.0.join("source/replacement.txt"),
                b"replacement not authorized",
            )
            .unwrap();
            let replacement = absolute(&f.0.join("source"), false).unwrap();
            assert!(require_identity(&replacement, &approved).is_err());
            require_identity(&root, &approved).unwrap();
            let pinned = entries(&root, "", 0).unwrap();
            assert_eq!(pinned.entries.len(), 1);
            assert_eq!(pinned.entries[0].name, "original.txt");
        }
        #[test]
        fn picker_cancellation_leaves_manifest_and_grants_unchanged() {
            let f = Fixture::new();
            let mut store = Store::open(&f.0).unwrap();
            let g = grant(&f.0.join("mock-source"));
            let id = g.row.id.clone();
            store.manifest.grants.push(g);
            store.save().unwrap();
            let manifest = f.0.join("data-access/manifest.json");
            let before = fs::read(&manifest).unwrap();
            let mut callback_ran = false;
            let result = finish_selection(None, |_| {
                callback_ran = true;
                store.revoke(&id)?;
                Ok(None)
            })
            .unwrap();
            assert!(result.is_none());
            assert!(!callback_ran);
            assert_eq!(store.manifest.grants.len(), 1);
            assert_eq!(fs::read(&manifest).unwrap(), before);
            // Cancellation must also avoid initializing previously absent storage.
            let absent = f.0.join("never-created-store");
            assert!(finish_selection(None, |_| {
                let _ = Store::open(&absent)?;
                Ok(None)
            })
            .unwrap()
            .is_none());
            assert!(!absent.exists());
        }
        #[test]
        fn concurrent_manifest_updates_are_serialized_without_lost_grants() {
            let f = Fixture::new();
            let base = f.0.clone();
            let workers: Vec<_> = (0..8)
                .map(|i| {
                    let base = base.clone();
                    std::thread::spawn(move || {
                        let mut store = Store::open(&base).unwrap();
                        store
                            .manifest
                            .grants
                            .push(grant(&base.join(format!("mock-source-{i}"))));
                        store.save().unwrap();
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
            let store = Store::open(&f.0).unwrap();
            assert_eq!(store.manifest.grants.len(), 8);
        }
        #[test]
        fn failed_manifest_commit_cleans_only_new_private_copy() {
            let f = Fixture::new();
            let root = f.source();
            fs::write(f.0.join("source/one.txt"), b"fixture only").unwrap();
            let mut s = Store::open(&f.0).unwrap();
            s.manifest.version = 999; // Force a pre-commit validation error.
            assert!(s.copy(&root, "/mock/source", "one.txt").is_err());
            assert_eq!(fs::read_dir(f.0.join("imports")).unwrap().count(), 0);
            assert!(s.manifest.imports.is_empty());
            assert_eq!(
                fs::read(f.0.join("source/one.txt")).unwrap(),
                b"fixture only"
            );
        }
        #[test]
        fn authorization_inventory_does_not_probe_source() {
            let f = Fixture::new();
            let mut s = Store::open(&f.0).unwrap();
            s.manifest
                .grants
                .push(grant(&f.0.join("never-created-source")));
            s.save().unwrap();
            drop(s);
            let s = Store::open(&f.0).unwrap();
            assert_eq!(s.manifest.grants.len(), 1);
            assert!(!f.0.join("never-created-source").exists());
        }
    }
}
