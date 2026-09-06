// Where a failure is written down.
//
// The engine used to say what the provider said on stderr and nowhere else, and an app launched
// from the Finder has no stderr anybody will ever read, so a report of "sync failed" arrived with
// nothing behind it. Every failure now also goes here, one line each, to `margin-mail.log` in the
// app data directory: a pass the provider refused, a body that would not come, a command the
// frontend called that answered with an error, an exception the webview caught. The file is kept
// to a size a person can open in a text editor. It is diagnostics rather than a feature: nothing
// in the app reads it back.
//
// This is Mailspring's `mailsync-<account>.log` in spirit: every caught exception, with what it
// was and who it belonged to, so that the next "it says sync failed" comes with the sentence
// behind it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const FILE: &str = "margin-mail.log";

/// Past this the file is cut back to its newer half, from a line boundary, before the next line
/// goes on. A quarter of a megabyte is a few thousand failures, which is more history than a bug
/// report needs and still opens in a text editor without a wait.
pub const CAP_BYTES: u64 = 256 * 1024;

/// A line longer than this is a stack trace or a body that landed in an error by mistake, and it
/// is cut, because one of those can be the whole cap.
const LINE_CAP: usize = 2_000;

static PATH: OnceLock<PathBuf> = OnceLock::new();
static WRITING: Mutex<()> = Mutex::new(());

/// Names the directory. Called once by whoever is handed the app data directory first; before
/// that, and in `cargo test`, a note goes to stderr alone.
pub fn init(dir: &Path) {
    let _ = PATH.set(dir.join(FILE));
}

pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}

/// One line, on stderr for anybody watching and in the file for everybody else. `who` is the
/// account id for anything the engine did on an account's behalf, and an area (`ui`, `bodies`,
/// `notify`) for anything that was nobody's in particular.
pub fn note(who: &str, line: &str) {
    let line = trim(line);
    eprintln!("[{who}] {line}");
    let Some(path) = PATH.get() else { return };
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let entry = format!("{stamp} {who} {line}\n");
    let _held = WRITING.lock();
    let _ = append(path, &entry, CAP_BYTES);
}

fn trim(line: &str) -> String {
    let flat = line.replace(['\n', '\r'], " ");
    if flat.chars().count() <= LINE_CAP {
        return flat;
    }
    let mut cut: String = flat.chars().take(LINE_CAP).collect();
    cut.push_str(" [cut]");
    cut
}

fn append(path: &Path, entry: &str, cap: u64) -> std::io::Result<()> {
    use std::io::Write;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size + entry.len() as u64 > cap {
        let held = std::fs::read(path).unwrap_or_default();
        let from = held.len().saturating_sub((cap / 2) as usize);
        let cut = held[from..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|at| from + at + 1)
            .unwrap_or(from);
        let mut kept = held[cut..].to_vec();
        kept.extend_from_slice(entry.as_bytes());
        return std::fs::write(path, kept);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(entry.as_bytes())
}

/// The frontend's way in. Every command that answers with an error is written down by the IPC
/// wrapper before the error reaches a toast, and the webview's own uncaught errors come the same
/// way, so the file holds what the person saw and not only what the engine did.
#[tauri::command]
pub fn log_note(who: String, line: String) {
    note(&who, &line);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "margin-mail-log-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&dir);
        dir
    }

    #[test]
    fn lines_go_on_the_end_and_the_file_never_outgrows_its_cap() {
        let path = scratch("cap");
        let line = "2026-09-05T10:00:00Z acct other: Gmail label change failed (400): no\n";
        for _ in 0..200 {
            append(&path, line, 2_048).expect("append");
        }
        let held = std::fs::read_to_string(&path).expect("the file");
        assert!(held.len() as u64 <= 2_048, "was {}", held.len());
        assert!(held.ends_with(line));
        // Cut on a line boundary, so the first line is whole rather than a tail of one.
        assert!(held.starts_with("2026-"), "{:?}", &held[..40]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_note_before_init_is_not_an_error() {
        // Nothing to assert beyond not panicking: in tests the path is never set.
        note("acct", "network: the train went into a tunnel");
    }

    #[test]
    fn a_line_is_one_line_and_a_long_one_is_cut() {
        assert_eq!(trim("a\nb\r\nc"), "a b  c");
        let long = "x".repeat(LINE_CAP + 50);
        let cut = trim(&long);
        assert!(cut.ends_with(" [cut]"));
        assert_eq!(cut.chars().count(), LINE_CAP + " [cut]".len());
    }
}
