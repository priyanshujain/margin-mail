// The two filesystem chores every module needs and nobody should write twice.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::Manager;

pub fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Write through a sibling temporary file and rename over the target.
///
/// Ported from margin's `project.rs`. Every settings write and every keymap write goes through it,
/// because the alternative is a truncated `settings.json` after a crash mid-write, and a truncated
/// settings file is an app that opens on the welcome screen with the accounts still on disk.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("that path has no directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    let temp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    {
        let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("settings.json");

        atomic_write(&path, b"{\"one\":1}").expect("first write");
        atomic_write(&path, b"{\"two\":2}").expect("second write");

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"two\":2}");
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["settings.json".to_string()]);
    }
}
