// The device id: made once per installation, kept in `state.meta`, and the reason a per-device
// sequence needs no coordination. Two devices never share a sequence, so appending is a local act
// and merging two logs is only ever a union.
//
// It is also the tie break in the merge, which is why it has to be stable for the life of the
// installation rather than made fresh per session: a device that renamed itself every launch would
// resolve the same pair of events differently on Tuesday than it did on Monday.

use rand::RngCore;
use rusqlite::{Connection, OptionalExtension};

pub const DEVICE_KEY: &str = "device-id";

pub fn device_id(conn: &Connection) -> Result<String, String> {
    if let Some(held) = meta_get(conn, DEVICE_KEY)? {
        if !held.is_empty() {
            return Ok(held);
        }
    }
    let fresh = fresh_device_id();
    meta_set(conn, DEVICE_KEY, &fresh)?;
    Ok(fresh)
}

/// Sixty-four random bits, hexadecimal. It names one installation among a person's own handful, and
/// it is compared as text when two events land in the same millisecond, so nothing about it needs
/// to be meaningful.
fn fresh_device_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    })
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM state.meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO state.meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}
