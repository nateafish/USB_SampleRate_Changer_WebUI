use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) const STATE_ROOT: &str = "/data/adb/usb_samplerate_changer_webui";
/// Volatile operation logs live outside the persistent Magisk state.  The
/// settings and lock remain under [`STATE_ROOT`], while logs can be safely
/// discarded on reboot.
pub(crate) const LOG_ROOT: &str = "/data/local/tmp/usb_samplerate_changer_webui";
pub(crate) const CORE_DIR: &str = "core";

pub(crate) fn module_dir() -> Result<PathBuf, String> {
    let executable =
        env::current_exe().map_err(|error| format!("cannot resolve executable: {error}"))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve module directory".to_string())
}

pub(crate) fn state_root() -> PathBuf {
    PathBuf::from(STATE_ROOT)
}

pub(crate) fn log_root() -> PathBuf {
    PathBuf::from(LOG_ROOT)
}

pub(crate) fn ensure_state_layout() -> Result<(), String> {
    let root = state_root();
    fs::create_dir_all(&root).map_err(|error| format!("cannot create state directory: {error}"))?;
    set_mode(&root, 0o700)?;
    let logs = log_root();
    fs::create_dir_all(&logs).map_err(|error| format!("cannot create log directory: {error}"))?;
    set_mode(&logs, 0o700)?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| format!("cannot set permissions on {}: {error}", path.display()))
}

pub(crate) fn atomic_write(path: &Path, content: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid output path: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name().and_then(OsStr::to_str).unwrap_or("output"),
        std::process::id(),
        sequence
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .map_err(|error| format!("cannot create {}: {error}", temp.display()))?;
    if let Err(error) = file.write_all(content).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot write {}: {error}", temp.display()));
    }
    set_mode(&temp, mode)?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("cannot replace {}: {error}", path.display())
    })?;

    // Persist the directory entry as well as the file contents. This keeps
    // the rename durable across abrupt Android reboots.
    let directory = OpenOptions::new()
        .read(true)
        .open(parent)
        .map_err(|error| format!("cannot open {} for sync: {error}", parent.display()))?;
    directory
        .sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", parent.display()))
}
