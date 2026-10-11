//! Native folder consent and balanced URL scopes. A saved bookmark never falls
//! back to ordinary path access when its scope cannot be activated. Fresh picker
//! grants are retained for this viewer process only, and released on revocation.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::Bool;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{
    NSData, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSUTF8StringEncoding,
    NSURL,
};

type Result<T> = std::result::Result<T, String>;

pub struct Scope {
    pub path: String,
    pub refreshed: Option<Vec<u8>>,
    url: Retained<NSURL>,
    // Separate ownership: the picker supplies implicit access; successful
    // explicit startAccess calls supply one additional reference count.
    explicit_started: bool,
    picker_access: bool,
    root: Option<std::fs::File>,
}

impl Scope {
    pub fn root(&self) -> &std::fs::File {
        self.root
            .as_ref()
            .expect("validated scope owns its root descriptor")
    }

    fn selected(url: Retained<NSURL>) -> Result<Self> {
        // Own the panel-provided access immediately, including validation errors.
        let mut scope = Self {
            path: String::new(),
            refreshed: None,
            url,
            explicit_started: false,
            picker_access: true,
            root: None,
        };
        scope.path = absolute_utf8_path(&scope.url)?;
        super::selected_path(&scope.path, &super::home()?)?;
        // No explicit start is needed for an NSOpenPanel URL. Keep its implicit
        // access alive until the saved grant is revoked or the process exits.
        scope.root = Some(super::fs_safe::absolute(Path::new(&scope.path), false)?);
        Ok(scope)
    }

    fn restored(url: Retained<NSURL>) -> Result<Self> {
        let path = absolute_utf8_path(&url)?;
        super::selected_path(&path, &super::home()?)?;
        // SAFETY: retained NSURL; each successful start is immediately owned.
        let started = unsafe { url.startAccessingSecurityScopedResource() };
        if !started {
            // NO is not a general permission check. For this broker it means
            // restored scoped authority is unproven: do NOT open/stat the path.
            return Err("此 App 未能恢复目录的安全作用域；请通过系统选择器重新选择该目录。未尝试直接路径读取，未申请完全磁盘访问。".into());
        }
        let mut scope = Self {
            path,
            refreshed: None,
            url,
            explicit_started: true,
            picker_access: false,
            root: None,
        };
        scope.root = Some(super::fs_safe::absolute(Path::new(&scope.path), false)?);
        Ok(scope)
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Close the descriptor BEFORE ending authority, including error paths.
        self.root.take();
        if self.explicit_started {
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
        if self.picker_access {
            // Apple documents panel URL access as already started. This owns
            // that separate grant; it is not a stop for a failed explicit start.
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

fn live() -> &'static Mutex<HashMap<String, Arc<Scope>>> {
    static LIVE: OnceLock<Mutex<HashMap<String, Arc<Scope>>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn remember(id: &str, scope: Arc<Scope>) -> Result<()> {
    live()
        .lock()
        .map_err(|_| "Directory scope registry unavailable")?
        .insert(id.into(), scope);
    Ok(())
}

pub fn forget(id: &str) {
    // A poisoned lock must still release the grant; no source I/O here.
    live().lock().unwrap_or_else(|e| e.into_inner()).remove(id);
}

/// Only this viewer's AppKit main thread may present this panel.
pub fn choose() -> Result<Option<super::SelectedDirectory>> {
    let mtm =
        MainThreadMarker::new().ok_or("Folder selection must run on the viewer's main thread")?;
    autoreleasepool(|_| {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(true);
        panel.setCanChooseFiles(false);
        panel.setAllowsMultipleSelection(false);
        panel.setCanCreateDirectories(false);
        // No default protected path, speculative directory probes, or helper.
        let response = panel.runModal();
        if response == NSModalResponseCancel {
            return Ok(None);
        }
        if response != NSModalResponseOK {
            return Err("macOS could not complete folder selection".into());
        }
        let url = panel.URL().ok_or("Folder selection returned no URL")?;
        let scope = Arc::new(Scope::selected(url)?);
        let bookmark = read_only_bookmark(&scope.url)?;
        let identity = super::fs_safe::identity(scope.root())?;
        Ok(Some(super::SelectedDirectory {
            path: scope.path.clone(),
            bookmark,
            identity,
            active: scope,
        }))
    })
}

/// Use held picker authority in this process, otherwise strictly restore the
/// saved bookmark. No UI, mounting, ordinary bookmark or saved-path fallback.
pub fn resolve(
    id: &str,
    bookmark: &[u8],
    expected_path: &str,
    expected_identity: &super::DirectoryIdentity,
) -> Result<Arc<Scope>> {
    if let Some(scope) = live()
        .lock()
        .map_err(|_| "Directory scope registry unavailable")?
        .get(id)
        .cloned()
    {
        if scope.path != expected_path {
            return Err("Stored directory differs from selected authority; choose again".into());
        }
        super::fs_safe::require_identity(scope.root(), expected_identity)?;
        return Ok(scope);
    }
    if bookmark.is_empty() {
        return Err("Security-scoped bookmark is empty".into());
    }
    autoreleasepool(|_| {
        let data = NSData::with_bytes(bookmark);
        let mut stale = Bool::NO;
        let options = NSURLBookmarkResolutionOptions::WithSecurityScope
            | NSURLBookmarkResolutionOptions::WithoutUI
            | NSURLBookmarkResolutionOptions::WithoutMounting;
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data, options, None, &mut stale,
            )
        }
        .map_err(|e| {
            format!("Cannot resolve security-scoped bookmark; choose the folder again: {e}")
        })?;
        let path = absolute_utf8_path(&url)?;
        super::selected_path(&path, &super::home()?)?;
        if path != expected_path {
            return Err("Authorized folder moved; remove it and choose again".into());
        }
        let mut scope = Scope::restored(url)?;
        super::fs_safe::require_identity(scope.root(), expected_identity)?;
        // Identity/path checks precede refresh, which may query resource info.
        if stale.as_bool() {
            scope.refreshed = Some(read_only_bookmark(&scope.url)?);
        }
        Ok(Arc::new(scope))
    })
}

fn read_only_bookmark(url: &NSURL) -> Result<Vec<u8>> {
    let options = NSURLBookmarkCreationOptions::WithSecurityScope
        | NSURLBookmarkCreationOptions::SecurityScopeAllowOnlyReadAccess;
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            options, None, None,
        )
        .map_err(|e| {
            format!("此 App 无法创建只读授权书签；未保存授权，也不会改用普通路径读取：{e}")
        })?;
    let bytes = data.to_vec();
    if bytes.is_empty() {
        return Err("macOS returned an empty security-scoped bookmark".into());
    }
    Ok(bytes)
}

fn absolute_utf8_path(url: &NSURL) -> Result<String> {
    if !url.isFileURL() {
        return Err("Bookmark or selection is not a file URL".into());
    }
    let path = url.path().ok_or("File URL has no path")?;
    let data = path
        .dataUsingEncoding_allowLossyConversion(NSUTF8StringEncoding, false)
        .ok_or("File URL path cannot be represented as UTF-8")?;
    let path = String::from_utf8(data.to_vec()).map_err(|_| "File URL path is not valid UTF-8")?;
    if !Path::new(&path).is_absolute() || path.contains('\0') {
        return Err("File URL path must be absolute and contain no NUL".into());
    }
    Ok(path)
}
