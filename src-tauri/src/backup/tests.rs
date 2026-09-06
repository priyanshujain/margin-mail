// The backup over a store that is a map in memory, which is the whole reason the store trait is
// three operations.
//
// Nothing here touches a network, a Google account or a bucket. What is being tested is the part
// that would be wrong in a way nobody notices: that two devices which never met land on the same
// tables, that the ciphertext is ciphertext, and that a pass cut off halfway leaves the store in a
// state the next pass can carry on from.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::db;
use crate::dto::{Destination, Person, Pile};
use crate::state::journal::{self, Payload};
use crate::state::{device, merge, write};

use super::crypto::{self, Key};
use super::store::BackupStore;
use super::{account_hash, pass, phrase, r2, Journal, Pass};

// -- the harness -------------------------------------------------------------------------------

/// A data directory: one pair of in-memory databases, with a device id of its own.
struct Device(Mutex<Connection>);

impl Journal for Device {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        f(&conn)
    }
}

impl Device {
    fn new() -> Device {
        Device(Mutex::new(
            db::memory().expect("a pair of in-memory databases"),
        ))
    }

    fn on<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        let conn = self.0.lock().expect("the connection");
        f(&conn)
    }

    fn id(&self) -> String {
        self.on(|conn| device::device_id(conn).expect("a device id"))
    }

    /// Every materialised table, row by row, in an order that does not depend on how they were
    /// written. Two devices have converged when these are equal.
    fn snapshot(&self) -> Vec<String> {
        self.on(|conn| {
            let mut out = Vec::new();
            for table in TABLES {
                let mut stmt = conn
                    .prepare(&format!("SELECT * FROM state.{table}"))
                    .expect("prepare");
                let columns = stmt.column_count();
                let rows = stmt
                    .query_map([], |row| {
                        let mut cells = Vec::new();
                        for at in 0..columns {
                            cells.push(format!("{:?}", row.get::<_, rusqlite::types::Value>(at)?));
                        }
                        Ok(cells.join("|"))
                    })
                    .expect("query");
                let mut table_rows: Vec<String> = rows
                    .map(|row| format!("{table}: {}", row.expect("row")))
                    .collect();
                table_rows.sort();
                out.extend(table_rows);
            }
            out
        })
    }
}

const TABLES: [&str; 11] = [
    "sender_rules",
    "piles",
    "snoozes",
    "notes",
    "renames",
    "merges",
    "clips",
    "thread_flags",
    "contacts",
    "prefs",
    "markers",
];

/// A store that is a map. `put` replaces a whole value under a lock, so a name never exists with
/// half a segment behind it, which is the property both real stores get from writing a blob in one
/// request.
#[derive(Default)]
struct Memory {
    blobs: Mutex<BTreeMap<String, Vec<u8>>>,
    /// How many more puts will be accepted before the store starts refusing, for the pass that gets
    /// cut off halfway.
    allowed: Mutex<Option<usize>>,
}

impl Memory {
    fn names(&self) -> Vec<String> {
        self.blobs.lock().expect("blobs").keys().cloned().collect()
    }

    fn blob(&self, name: &str) -> Vec<u8> {
        self.blobs
            .lock()
            .expect("blobs")
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} is not in the store"))
    }

    fn breaks_after(&self, puts: usize) {
        *self.allowed.lock().expect("allowed") = Some(puts);
    }

    fn mended(&self) {
        *self.allowed.lock().expect("allowed") = None;
    }
}

impl BackupStore for Memory {
    fn put(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<(), String>> + Send {
        let name = name.to_string();
        let bytes = bytes.to_vec();
        async move {
            let mut allowed = self.allowed.lock().map_err(|e| e.to_string())?;
            match allowed.as_mut() {
                Some(0) => return Err("the network went away".to_string()),
                Some(left) => *left -= 1,
                None => {}
            }
            self.blobs
                .lock()
                .map_err(|e| e.to_string())?
                .insert(name, bytes);
            Ok(())
        }
    }

    fn get(&self, name: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send {
        let name = name.to_string();
        async move {
            self.blobs
                .lock()
                .map_err(|e| e.to_string())?
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("{name} is not in the store"))
        }
    }

    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>, String>> + Send {
        let prefix = prefix.to_string();
        async move {
            Ok(self
                .blobs
                .lock()
                .map_err(|e| e.to_string())?
                .keys()
                .filter(|name| name.starts_with(&prefix))
                .cloned()
                .collect())
        }
    }
}

const ACCOUNT: &str = "ana@example.com";

fn hash() -> String {
    account_hash(ACCOUNT)
}

fn key() -> Key {
    Key::from_bytes([7u8; 32])
}

fn run(device: &Device, store: &Memory, key: &Key) -> Pass {
    block_on(pass(device, store, key, &hash())).expect("a pass")
}

fn ana() -> Person {
    Person {
        name: Some("Ana".to_string()),
        address: ACCOUNT.to_string(),
    }
}

// -- what goes up ------------------------------------------------------------------------------

#[test]
fn what_goes_up_comes_back_byte_for_byte_after_decryption() {
    let device = Device::new();
    let store = Memory::default();
    device.on(|conn| {
        write::set_rule(
            conn,
            "landlord@example.com",
            false,
            Destination::Inbox,
            None,
        )
        .expect("a rule");
        write::add_note(conn, "thread-1", "the roof is leaking", None).expect("a note");
    });

    let pass = run(&device, &store, &key());
    assert_eq!(pass.uploaded, 1);
    assert_eq!(pass.downloaded, 0);

    let names = store.names();
    assert_eq!(names.len(), 1);
    let expected = device.on(|conn| {
        let id = device::device_id(conn).expect("device");
        merge::encode(&merge::export(conn, &id, 0).expect("export")).expect("encode")
    });
    let plain = crypto::open(&key(), &names[0], &store.blob(&names[0])).expect("decrypt");
    assert_eq!(String::from_utf8(plain).expect("utf8"), expected);

    // And a second pass finds nothing new to send, because the store is what says what it holds.
    assert_eq!(run(&device, &store, &key()).uploaded, 0);
}

#[test]
fn the_store_holds_ciphertext_and_names_that_say_nothing() {
    let device = Device::new();
    let store = Memory::default();
    device.on(|conn| {
        write::set_rule(conn, "landlord@example.com", false, Destination::Feed, None)
            .expect("a rule");
        write::add_note(conn, "thread-roof", "the roof is leaking", None).expect("a note");
        write::add_clip(
            conn,
            "thread-roof",
            "message-1",
            "the plumber comes on Thursday",
            &ana(),
            "About the roof",
        )
        .expect("a clip");
    });
    run(&device, &store, &key());

    let secrets = [
        "landlord@example.com",
        "the roof is leaking",
        "the plumber comes on Thursday",
        "About the roof",
        "thread-roof",
        "note",
        "clip",
        "sender-rule",
        ACCOUNT,
    ];
    for name in store.names() {
        let blob = store.blob(&name);
        let text = String::from_utf8_lossy(&blob).to_string();
        for secret in secrets {
            assert!(
                !text.contains(secret),
                "{secret} is readable in the segment at {name}"
            );
            assert!(!name.contains(secret), "{secret} is readable in {name}");
        }
    }
}

#[test]
fn a_segment_moved_to_another_devices_folder_does_not_open() {
    let device = Device::new();
    let store = Memory::default();
    device.on(|conn| write::set_pile(conn, "thread-1", Pile::ReplyLater).expect("a pile"));
    run(&device, &store, &key());

    let name = store.names().into_iter().next().expect("a segment");
    let blob = store.blob(&name);
    assert!(crypto::open(&key(), &name, &blob).is_ok());

    let moved = name.replace(&device.id(), "somebody-elses-device");
    assert!(
        crypto::open(&key(), &moved, &blob).is_err(),
        "a segment is authenticated under the name it was written to"
    );
}

// -- two devices -------------------------------------------------------------------------------

/// The done check for this package. Two data directories, each with its own device id, each holding
/// decisions the other never saw, meeting only through a store, in both orders.
#[test]
fn two_devices_converge_on_identical_tables_in_either_order() {
    for reversed in [false, true] {
        let one = Device::new();
        let two = Device::new();
        let store = Memory::default();

        one.on(|conn| {
            write::set_rule(
                conn,
                "landlord@example.com",
                false,
                Destination::Inbox,
                None,
            )
            .expect("a rule");
            write::add_note(conn, "thread-roof", "the roof is leaking", None).expect("a note");
            write::set_pile(conn, "thread-roof", Pile::ReplyLater).expect("a pile");
            // The same key on both devices, decided at two different moments, so that convergence
            // here means agreeing about a conflict rather than merely taking a union.
            journal::append_at(
                conn,
                5_000,
                "thread-shared",
                &Payload::Rename {
                    name: Some("what one called it".into()),
                },
            )
            .expect("a rename");
        });
        two.on(|conn| {
            write::set_rule(
                conn,
                "bank@example.com",
                false,
                Destination::PaperTrail,
                None,
            )
            .expect("a rule");
            write::set_snooze(
                conn,
                "thread-bill",
                9_000,
                crate::dto::SnoozeKind::Tomorrow,
                0,
            )
            .expect("a snooze");
            journal::append_at(
                conn,
                9_000,
                "thread-shared",
                &Payload::Rename {
                    name: Some("what two called it".into()),
                },
            )
            .expect("a rename");
        });

        let (first, second) = if reversed { (&two, &one) } else { (&one, &two) };
        run(first, &store, &key());
        run(second, &store, &key());
        // The first device has not seen the second's yet: it uploaded before there was anything to
        // fetch. One more pass each way is what "converged" means.
        run(first, &store, &key());

        assert_eq!(
            one.snapshot(),
            two.snapshot(),
            "the two devices disagree, reversed = {reversed}"
        );
        assert!(!one.snapshot().is_empty());
        // The later decision owns the shared key on both of them, whichever order they met in.
        assert!(one
            .snapshot()
            .iter()
            .any(|row| row.contains("what two called it")));
    }
}

#[test]
fn a_device_that_missed_a_month_catches_up_in_one_pass() {
    let store = Memory::default();
    let busy = Device::new();
    let kept_up = Device::new();
    let away = Device::new();

    // Four weeks of decisions, with the device that was on backing up after each of them.
    for week in 0..4 {
        busy.on(|conn| {
            write::set_rule(
                conn,
                &format!("sender-{week}@example.com"),
                false,
                Destination::Feed,
                None,
            )
            .expect("a rule");
            write::add_note(conn, &format!("thread-{week}"), "worth remembering", None)
                .expect("a note");
        });
        run(&busy, &store, &key());
        run(&kept_up, &store, &key());
    }

    let pass = run(&away, &store, &key());
    assert_eq!(pass.downloaded, 4, "one pass, four segments, nothing else");
    assert_eq!(away.snapshot(), kept_up.snapshot());
    assert_eq!(away.snapshot(), busy.snapshot());

    // And having caught up, it asks for nothing the next time.
    assert_eq!(run(&away, &store, &key()).downloaded, 0);
}

#[test]
fn a_second_device_attaches_with_the_phrase_and_reads_what_the_first_wrote() {
    let written = phrase::generate().expect("a phrase");
    assert_eq!(written.split_whitespace().count(), phrase::WORDS);
    let key = phrase::derive(&written).expect("the key");

    let store = Memory::default();
    let first = Device::new();
    first.on(|conn| {
        write::add_note(conn, "thread-roof", "the roof is leaking", None).expect("a note");
        write::set_rule(
            conn,
            "landlord@example.com",
            false,
            Destination::Inbox,
            None,
        )
        .expect("a rule");
    });
    run(&first, &store, &key);

    // The second device has the words off a piece of paper, typed the way somebody types.
    let second = Device::new();
    let typed = format!("  {}  ", written.to_uppercase());
    let second_key = phrase::derive(&typed).expect("the same key");
    run(&second, &store, &second_key);

    assert_eq!(second.snapshot(), first.snapshot());
    assert!(!second.snapshot().is_empty());
}

#[test]
fn a_wrong_phrase_fails_clearly_rather_than_producing_garbage() {
    let store = Memory::default();
    let first = Device::new();
    let right = phrase::derive(&phrase::generate().expect("a phrase")).expect("a key");
    first.on(|conn| {
        write::add_note(conn, "thread-roof", "the roof is leaking", None).expect("a note");
    });
    run(&first, &store, &right);

    let second = Device::new();
    let wrong = phrase::derive(&phrase::generate().expect("another phrase")).expect("a key");
    let refused = block_on(pass(&second, &store, &wrong, &hash())).expect_err("the wrong key");
    assert!(
        refused.contains("recovery phrase"),
        "the failure has to say what went wrong: {refused}"
    );
    assert!(
        second.snapshot().is_empty(),
        "nothing half decrypted landed in the tables"
    );

    // A mistyped word is caught by the wordlist before any of that, and names itself.
    let mistyped = phrase::normalise(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon zzzz",
    );
    assert!(mistyped
        .expect_err("not a phrase")
        .contains("recovery phrase"));
}

// -- a pass that does not finish -----------------------------------------------------------------

#[test]
fn an_interrupted_upload_leaves_whole_segments_and_the_next_pass_carries_on() {
    let store = Memory::default();
    let device = Device::new();
    // Two segments' worth, so there is a second put to refuse.
    device.on(|conn| {
        for at in 0..600 {
            write::set_marker(conn, &format!("place-{at}"), at as i64).expect("a marker");
        }
    });

    store.breaks_after(1);
    let cut_off = block_on(pass(&device, &store, &key(), &hash())).expect_err("the network went");
    assert!(cut_off.contains("network"));

    // What is on the store is one whole segment, not one and a half.
    let names = store.names();
    assert_eq!(names.len(), 1);
    let records = {
        let plain = crypto::open(&key(), &names[0], &store.blob(&names[0])).expect("decrypt");
        merge::decode(&String::from_utf8(plain).expect("utf8")).expect("decode")
    };
    assert_eq!(records.len(), super::SEGMENT_RECORDS);
    assert_eq!(records.first().expect("first").seq, 1);

    store.mended();
    let mended = run(&device, &store, &key());
    assert_eq!(
        mended.uploaded, 1,
        "the segment that landed is not sent again"
    );
    assert_eq!(store.names().len(), 2);

    let other = Device::new();
    run(&other, &store, &key());
    assert_eq!(other.snapshot(), device.snapshot());
}

// -- the names -----------------------------------------------------------------------------------

#[test]
fn an_account_folder_is_named_by_a_hash_rather_than_by_an_address() {
    let hash = account_hash(ACCOUNT);
    assert_eq!(hash.len(), 32);
    assert!(!hash.contains('@'));
    assert_eq!(
        hash,
        account_hash(" Ana@Example.COM "),
        "one address, one folder, whatever the casing"
    );
    assert_ne!(hash, account_hash("ana@example.org"));
}

#[test]
fn a_segment_name_carries_a_range_and_nothing_else() {
    let name = super::segment_name("abc", "device-1", 1, 500);
    assert_eq!(name, "abc/device-1/000000000001-000000000500.seg");
    assert_eq!(
        super::parse_name(&name),
        Some(("device-1".to_string(), 1, 500))
    );
    // Sorting names sorts the segments, which is why the numbers are padded.
    let mut names = vec![
        super::segment_name("abc", "device-1", 1001, 1500),
        super::segment_name("abc", "device-1", 1, 500),
        super::segment_name("abc", "device-1", 501, 1000),
    ];
    names.sort();
    assert_eq!(names[0], super::segment_name("abc", "device-1", 1, 500));
    assert_eq!(names[2], super::segment_name("abc", "device-1", 1001, 1500));

    assert_eq!(super::parse_name("abc/device-1/notes.txt"), None);
    assert_eq!(
        super::parse_name("abc/device-1/deeper/000000000001-000000000002.seg"),
        None
    );
}

// -- signature version four ------------------------------------------------------------------------

/// AWS publishes the signing key for these inputs, so this is the vector rather than a value read
/// off this implementation and pasted back in.
#[test]
fn the_signing_key_matches_the_published_derivation() {
    let key = r2::signing_key(
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        "20120215",
        "us-east-1",
        "iam",
    );
    let hex = key.iter().fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    });
    assert_eq!(
        hex,
        "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
    );
}

/// `get-vanilla` from AWS's own signature version four test suite, end to end through the same
/// function the uploads go through.
#[test]
fn the_authorization_header_matches_the_published_test_suite() {
    let header = r2::authorization(
        "AKIDEXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        "us-east-1",
        "service",
        "GET",
        "/",
        "",
        &[
            ("host", "example.amazonaws.com".to_string()),
            ("x-amz-date", "20150830T123600Z".to_string()),
        ],
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "20150830T123600Z",
    );
    assert_eq!(
        header,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
         SignedHeaders=host;x-amz-date, \
         Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
}

#[test]
fn a_bucket_listing_is_read_out_of_the_xml_it_arrives_in() {
    let body = "<?xml version=\"1.0\"?><ListBucketResult><IsTruncated>false</IsTruncated>\
        <Contents><Key>abc/device-1/000000000001-000000000500.seg</Key><Size>10</Size></Contents>\
        <Contents><Key>abc/device-2/000000000001-000000000004.seg</Key></Contents></ListBucketResult>";
    assert_eq!(
        r2::tags(body, "Key"),
        vec![
            "abc/device-1/000000000001-000000000500.seg".to_string(),
            "abc/device-2/000000000001-000000000004.seg".to_string()
        ]
    );
    assert!(r2::tags(body, "NextContinuationToken").is_empty());
}

#[test]
fn the_four_fields_are_checked_while_the_person_still_has_the_form_open() {
    let fields = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<std::collections::HashMap<_, _>>()
    };

    let good = r2::Config::from_fields(&fields(&[
        ("endpoint", "https://example.r2.cloudflarestorage.com/"),
        ("bucket", "margin"),
        ("accessKey", "key"),
        ("secret", "shh"),
    ]))
    .expect("four fields");
    assert_eq!(good.endpoint, "https://example.r2.cloudflarestorage.com");

    assert!(r2::Config::from_fields(&fields(&[
        ("endpoint", "http://example.com"),
        ("bucket", "margin"),
        ("accessKey", "key"),
        ("secret", "shh")
    ]))
    .is_err());
    assert!(r2::Config::from_fields(&fields(&[
        ("endpoint", "https://example.com"),
        ("bucket", "margin"),
        ("accessKey", "key")
    ]))
    .expect_err("no secret")
    .contains("secret"));
}
