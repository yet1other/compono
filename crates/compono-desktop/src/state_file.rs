//! Where a desktop application keeps its saved tree.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The system gave no directory for application state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoStateDir;

impl fmt::Display for NoStateDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no directory for application state")
    }
}
impl std::error::Error for NoStateDir {}

/// A file holding the bytes of [`Tree::save`](https://docs.rs/compono).
///
/// Each save writes a temporary file of its own in the same directory, syncs it
/// and renames it over the old one, so a reader sees the old bytes or the new
/// ones whole, even while several threads or processes save at once, and a crash
/// during a save leaves the previous state readable. The last rename wins.
///
/// Store only bytes that `Tree::save` returned. When it returns an error, for
/// example because the tree is poisoned, do not call [`StateFile::store`], so the
/// last good state stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    /// Uses the file at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Uses `state.bin` in the system's directory for the application `app`.
    ///
    /// That is `~/Library/Application Support/<app>` on macOS,
    /// `%LOCALAPPDATA%\<app>` on Windows, and `$XDG_STATE_HOME/<app>` or
    /// `~/.local/state/<app>` elsewhere.
    ///
    /// # Errors
    ///
    /// Returns [`NoStateDir`] when the environment names no such directory.
    pub fn in_app_dir(app: &str) -> Result<Self, NoStateDir> {
        let base = state_dir().ok_or(NoStateDir)?;
        Ok(Self::new(base.join(app).join("state.bin")))
    }

    /// Returns where the state lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the saved bytes, or `None` if nothing has been saved yet.
    ///
    /// # Errors
    ///
    /// Returns any error other than a missing file.
    pub fn load(&self) -> io::Result<Option<Vec<u8>>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Replaces the saved bytes, creating the directory if needed.
    ///
    /// The bytes go to a new temporary file next to the state file, which is
    /// synced and renamed over it. Then the directory is synced, so the rename
    /// survives a power cut, on every system but Windows, which cannot open a
    /// directory through the standard library and relies on its file system's
    /// journal instead. A save that fails removes its temporary file.
    ///
    /// # Errors
    ///
    /// Returns the error of creating, writing, renaming or syncing.
    pub fn store(&self, bytes: &[u8]) -> io::Result<()> {
        let dir = match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        fs::create_dir_all(dir)?;
        let (temporary, file) = create_temporary(&self.path)?;
        let written = write_synced(file, bytes).and_then(|()| fs::rename(&temporary, &self.path));
        if written.is_err() {
            // The old state stays. The half-written file is of no use.
            let _already_gone = fs::remove_file(&temporary);
            return written;
        }
        sync_dir(dir)
    }

    /// Removes the saved bytes. A missing file is not an error.
    ///
    /// # Errors
    ///
    /// Returns any error other than a missing file.
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// Creates a temporary file no other save uses, named after `path`, the process
/// and a counter, in the same directory so the rename stays on one file system.
fn create_temporary(path: &Path) -> io::Result<(PathBuf, fs::File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().unwrap_or_else(|| "state".as_ref());
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    // A file left by a crashed process with a reused id is skipped.
    for _ in 0..64 {
        let mut temporary = OsString::from(".");
        temporary.push(name);
        temporary.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = path.with_file_name(temporary);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

fn write_synced(mut file: fs::File, bytes: &[u8]) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

/// Makes a rename in `dir` durable.
#[cfg(not(windows))]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// Windows cannot open a directory without `FILE_FLAG_BACKUP_SEMANTICS`, which
/// the standard library does not set. NTFS journals the rename itself.
#[cfg(windows)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature on every system"
)]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value: &OsString| !value.is_empty())
        .map(PathBuf::from)
}

fn state_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        var("LOCALAPPDATA")
    } else {
        var("XDG_STATE_HOME").or_else(|| var("HOME").map(|home| home.join(".local").join("state")))
    }
}
