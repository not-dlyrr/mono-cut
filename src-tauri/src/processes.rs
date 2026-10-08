//! Native child ownership shared by synchronous media preparation and background jobs.
use std::{
    collections::HashMap,
    fs, io,
    path::{Component, Path, PathBuf},
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Default)]
struct Inner {
    closing: bool,
    next_id: u64,
    total_spawn_count: u64,
    children: HashMap<u64, Arc<Mutex<Child>>>,
    temporary_files: HashMap<u64, PathBuf>,
    asset_pins: HashMap<PathBuf, usize>,
}
#[derive(Debug, Default)]
pub struct ProcessRegistry {
    inner: Mutex<Inner>,
}
pub struct ManagedChild {
    owner: Arc<ProcessRegistry>,
    id: u64,
    child: Arc<Mutex<Child>>,
}
pub struct TemporaryFile {
    owner: Arc<ProcessRegistry>,
    id: u64,
    path: PathBuf,
}
/// A live media consumer owns this guard until it stops using the cached file.
/// Registration also supports destinations that have not been written yet.
#[derive(Debug)]
pub struct AssetPin {
    owner: Arc<ProcessRegistry>,
    key: PathBuf,
    path: PathBuf,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtectedAssets {
    pub file_count: usize,
    pub pin_count: usize,
    pub bytes: u64,
}
fn ordinary_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{unc}"));
        }
        if let Some(plain) = text.strip_prefix("\\\\?\\") {
            return PathBuf::from(plain);
        }
    }
    path
}
/// Resolve existing assets and existing parent directories so a pin taken before
/// an atomic cache rename has the same identity as its final, now-existing file.
fn asset_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return ordinary_path(canonical);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut clean = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(component.as_os_str()),
        }
    }
    let mut existing = clean.clone();
    let mut tail = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(|name| name.to_os_string()) else {
            break;
        };
        tail.push(name);
        if !existing.pop() {
            break;
        }
    }
    if let Ok(canonical) = existing.canonicalize() {
        let mut resolved = ordinary_path(canonical);
        for component in tail.into_iter().rev() {
            resolved.push(component);
        }
        resolved
    } else {
        ordinary_path(clean)
    }
}
fn asset_key(path: &Path) -> PathBuf {
    let resolved = asset_path(path);
    #[cfg(windows)]
    {
        return PathBuf::from(resolved.to_string_lossy().to_lowercase());
    }
    #[cfg(not(windows))]
    {
        resolved
    }
}
fn closing_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Media processing cancelled: application is closing",
    )
}
impl ProcessRegistry {
    pub fn pin_file(self: &Arc<Self>, path: impl AsRef<Path>) -> io::Result<AssetPin> {
        let path = asset_path(path.as_ref());
        let key = asset_key(&path);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.closing {
            return Err(closing_error());
        }
        *inner.asset_pins.entry(key.clone()).or_default() += 1;
        Ok(AssetPin {
            owner: self.clone(),
            key,
            path,
        })
    }
    pub fn is_protected(&self, path: impl AsRef<Path>) -> bool {
        let key = asset_key(path.as_ref());
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .asset_pins
            .contains_key(&key)
    }
    pub fn protected_assets(&self) -> ProtectedAssets {
        let pins: Vec<_> = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .asset_pins
            .iter()
            .map(|(path, count)| (path.clone(), *count))
            .collect();
        ProtectedAssets {
            file_count: pins.len(),
            pin_count: pins.iter().map(|(_, count)| count).sum(),
            bytes: pins
                .iter()
                .filter_map(|(path, _)| fs::metadata(path).ok().map(|meta| meta.len()))
                .fold(0, u64::saturating_add),
        }
    }
    /// Check and delete under the same ownership lock as pin registration. A
    /// stale prune candidate can never remove a newly protected playback asset.
    pub fn remove_unprotected_file(&self, path: impl AsRef<Path>) -> io::Result<bool> {
        let path = path.as_ref();
        let key = asset_key(path);
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.asset_pins.contains_key(&key) {
            return Ok(false);
        }
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    pub fn is_closing(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).closing
    }
    pub fn active_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .children
            .len()
    }
    pub fn total_spawn_count(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total_spawn_count
    }
    pub fn spawn(self: &Arc<Self>, command: &mut Command) -> io::Result<ManagedChild> {
        // Serialize spawn with shutdown so a process cannot escape registration at exit.
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.closing {
            return Err(closing_error());
        }
        let child = Arc::new(Mutex::new(command.spawn()?));
        inner.total_spawn_count += 1;
        inner.next_id += 1;
        let id = inner.next_id;
        inner.children.insert(id, child.clone());
        Ok(ManagedChild {
            owner: self.clone(),
            id,
            child,
        })
    }
    pub fn temporary_file(self: &Arc<Self>, path: PathBuf) -> io::Result<TemporaryFile> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.closing {
            return Err(closing_error());
        }
        inner.next_id += 1;
        let id = inner.next_id;
        inner.temporary_files.insert(id, path.clone());
        Ok(TemporaryFile {
            owner: self.clone(),
            id,
            path,
        })
    }
    /// Called only on actual app exit, after close interception has been resolved.
    pub fn shutdown(&self) {
        let (children, files) = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.closing = true;
            (
                inner.children.values().cloned().collect::<Vec<_>>(),
                inner.temporary_files.values().cloned().collect::<Vec<_>>(),
            )
        };
        for handle in children {
            let mut child = handle.lock().unwrap_or_else(|e| e.into_inner());
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        for path in files {
            let _ = fs::remove_file(path);
        }
    }
}
impl AssetPin {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for AssetPin {
    fn drop(&mut self) {
        let mut inner = self.owner.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = inner.asset_pins.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                inner.asset_pins.remove(&self.key);
            }
        }
    }
}
impl ManagedChild {
    pub fn take_stdout(&self) -> Option<ChildStdout> {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stdout
            .take()
    }
    pub fn take_stderr(&self) -> Option<ChildStderr> {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stderr
            .take()
    }
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_wait()
    }
    pub fn kill(&self) -> io::Result<()> {
        self.child.lock().unwrap_or_else(|e| e.into_inner()).kill()
    }
    pub fn wait(&self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            // Never hold the child lock while waiting, so shutdown can terminate it.
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        {
            let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.owner
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .children
            .remove(&self.id);
    }
}
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        self.owner
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .temporary_files
            .remove(&self.id);
    }
}
