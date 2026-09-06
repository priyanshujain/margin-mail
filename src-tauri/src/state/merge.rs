// Two devices' logs meeting.
//
// There is nothing to reconcile in the log itself. A device only ever appends under its own id, so
// no two of them can write the same (device, seq) and putting two logs together is a union. What
// needs deciding is which event owns a key once both are in one place, and that is the last writer
// wins in `journal`, which is a function of the events rather than of the order they were absorbed
// in. Absorbing the same segment twice, or absorbing yesterday's before today's, lands in the same
// place as absorbing them once in order.
//
// An event of a kind this build does not understand is still stored. It is not this device's to
// throw away: the device that wrote it is on a newer build of the same account, and this one may be
// asked for it later or may grow into understanding it after an update.

use rusqlite::Connection;

use super::journal::{self, Landing, Record, Report};

/// Every device that has ever written to this account's log, this one included.
pub fn devices(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT device_id FROM state.journal ORDER BY device_id ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// The last sequence held for each device, which is what a sync asks the backup store for more
/// than: everything above this, for every device, is what this one has not seen.
pub fn high_water(conn: &Connection) -> Result<Vec<(String, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT device_id, MAX(seq) FROM state.journal GROUP BY device_id
              ORDER BY device_id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// One device's events above a sequence, in sequence order, which is the shape of a segment going
/// up to the backup store.
pub fn export(conn: &Connection, device_id: &str, after_seq: i64) -> Result<Vec<Record>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT device_id, seq, at_ms, kind, key, payload FROM state.journal
              WHERE device_id = ?1 AND seq > ?2
              ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![device_id, after_seq], |row| {
            Ok(Record {
                device_id: row.get(0)?,
                seq: row.get(1)?,
                at_ms: row.get(2)?,
                kind: row.get(3)?,
                key: row.get(4)?,
                payload: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Takes in events from somewhere else: another device's segment, or this account's own log out of
/// a backup. Records already held are left alone rather than applied again, and the rest are landed
/// in the order the merge is defined by, so what comes out does not depend on how the caller
/// happened to have them stacked.
pub fn absorb(conn: &Connection, records: &[Record]) -> Result<Report, String> {
    let mut ordered: Vec<&Record> = records.iter().collect();
    ordered.sort_by(|a, b| a.order().cmp(&b.order()));

    journal::atomically(conn, |conn| {
        let mut report = Report::default();
        for record in ordered {
            if !journal::insert(conn, record)? {
                continue;
            }
            match journal::apply(conn, record)? {
                Landing::Applied => report.applied += 1,
                Landing::Older | Landing::Skipped => report.skip(&record.kind),
            }
        }
        Ok(report)
    })
}

/// What a segment holds. The store's business is a blob at a name, and the encryption's business is
/// the bytes, so the format of what is inside them is the log's own.
pub fn encode(records: &[Record]) -> Result<String, String> {
    serde_json::to_string(records).map_err(|e| e.to_string())
}

pub fn decode(raw: &str) -> Result<Vec<Record>, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}
