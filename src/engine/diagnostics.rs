//! Best-effort logging with a cached file handle in a user-writable directory.
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

fn candidate_dirs(
    override_dir: Option<OsString>,
    local_app_data: Option<OsString>,
    temp_dir: PathBuf,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = local_app_data.filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir).join("VibeClient").join("logs"));
    }
    dirs.push(temp_dir.join("VibeClient").join("logs"));
    dirs
}

fn open_log(dir: &Path) -> std::io::Result<File> {
    fs::create_dir_all(dir)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("client.log"))
}

pub fn log_msg(msg: &str) {
    let file = LOG_FILE.get_or_init(|| {
        let dirs = candidate_dirs(
            std::env::var_os("VIBE_LOG_DIR"),
            std::env::var_os("LOCALAPPDATA"),
            std::env::temp_dir(),
        );
        Mutex::new(dirs.iter().find_map(|dir| open_log(dir).ok()))
    });
    // Logging must not block a render callback on another logging thread.
    if let Ok(mut slot) = file.try_lock() {
        if let Some(file) = slot.as_mut() {
            let _ = writeln!(file, "{}", msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_precedes_user_and_temp_paths() {
        let dirs = candidate_dirs(
            Some("custom".into()),
            Some("user".into()),
            PathBuf::from("temp"),
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("custom"),
                PathBuf::from("user").join("VibeClient").join("logs"),
                PathBuf::from("temp").join("VibeClient").join("logs"),
            ]
        );
    }

    #[test]
    fn absent_or_empty_settings_use_temp() {
        let expected = vec![PathBuf::from("temp").join("VibeClient").join("logs")];
        assert_eq!(candidate_dirs(None, None, PathBuf::from("temp")), expected);
        assert_eq!(
            candidate_dirs(Some("".into()), Some("".into()), PathBuf::from("temp")),
            expected
        );
    }

    #[test]
    fn cached_handle_appends_multiple_messages() {
        let dir = std::env::temp_dir().join(format!(
            "vibe-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        {
            let mut file = open_log(&dir).unwrap();
            writeln!(file, "first").unwrap();
            writeln!(file, "second").unwrap();
        }
        assert_eq!(
            fs::read_to_string(dir.join("client.log")).unwrap(),
            "first\nsecond\n"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
