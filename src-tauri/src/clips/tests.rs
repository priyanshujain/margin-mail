// Clips and All files, over a pair of in-memory databases. Nothing here fetches anything, which is
// the point of the place: the cards are built from the index the sync already wrote.

use rusqlite::Connection;

use crate::db;
use crate::state;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

/// One file, and the message and thread it hangs off. A struct rather than nine arguments, which
/// is what `screener::tests` does with a sender for the same reason.
struct File<'a> {
    id: &'a str,
    address: &'a str,
    subject: &'a str,
    at: i64,
    filename: &'a str,
    mime_type: &'a str,
    size: i64,
    /// Referenced from the body by `cid:` rather than listed as a chip.
    inline: bool,
}

fn with_file(conn: &Connection, file: &File<'_>) -> String {
    let File {
        id,
        address,
        subject,
        at,
        filename,
        mime_type,
        size,
        inline,
    } = *file;
    let key = format!("<{id}@example>");
    let tid = format!("t-{id}");
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox, has_attachment)
         VALUES (?1, ?2, ?3, 1, 0, ?4, 'snippet', 'Someone', ?5, 1, 1)",
        rusqlite::params![tid, key, at, subject, address],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_name, from_address, subject, snippet, hydrated,
                               has_attachment, labels)
         VALUES (?1, ?2, ?3, ?3, ?4, 'Someone', ?5, ?6, 'snippet', 1, 1, '[\"INBOX\"]')",
        rusqlite::params![format!("m-{id}"), tid, key, at, address, subject],
    )
    .expect("a message");
    conn.execute(
        "INSERT INTO attachments (id, message_id, part_id, filename, mime_type, size, inline)
         VALUES (?1, ?2, '2', ?3, ?4, ?5, ?6)",
        rusqlite::params![
            format!("a-{id}"),
            format!("m-{id}"),
            filename,
            mime_type,
            size,
            inline as i64
        ],
    )
    .expect("a file");
    key
}

fn pdf<'a>(id: &'a str, address: &'a str, subject: &'a str, at: i64) -> File<'a> {
    File {
        id,
        address,
        subject,
        at,
        filename: "quote.pdf",
        mime_type: "application/pdf",
        size: 90_000,
        inline: false,
    }
}

fn image<'a>(
    id: &'a str,
    address: &'a str,
    subject: &'a str,
    at: i64,
    filename: &'a str,
    size: i64,
    inline: bool,
) -> File<'a> {
    File {
        id,
        address,
        subject,
        at,
        filename,
        mime_type: "image/png",
        size,
        inline,
    }
}

// ---------------------------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------------------------

#[test]
fn a_clip_keeps_the_words_and_where_they_came_from() {
    let conn = open();
    let key = with_file(&conn, &pdf("one", "maya@example.org", "The kitchen quote", 300));

    let clip = super::save(&conn, "acct", &key, "m-one", "  the price holds until March  ")
        .expect("a clip");
    assert_eq!(clip.text, "the price holds until March");
    assert_eq!(clip.sender.address, "maya@example.org");
    assert_eq!(clip.subject, "The kitchen quote");
    assert_eq!(clip.thread_key, key);

    assert_eq!(
        state::read::clips(&conn, "acct", 50).expect("the clips").len(),
        1
    );

    state::write::delete_clip(&conn, &clip.id).expect("deleted");
    assert!(state::read::clips(&conn, "acct", 50).expect("the clips").is_empty());

    super::restore(&conn, &clip).expect("put back");
    let held = state::read::clips(&conn, "acct", 50).expect("the clips");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].text, "the price holds until March");
}

// ---------------------------------------------------------------------------------------------
// All files
// ---------------------------------------------------------------------------------------------

#[test]
fn a_file_is_categorised_from_its_type_and_its_extension() {
    for (mime_type, filename, expected) in [
        ("application/pdf", "quote.pdf", "pdfs"),
        ("image/png", "plan.png", "images"),
        ("text/calendar", "invite.ics", "invites"),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "costs.xlsx",
            "spreadsheets",
        ),
        ("application/vnd.ms-powerpoint", "deck.ppt", "presentations"),
        ("application/msword", "letter.doc", "documents"),
        ("application/zip", "photos.zip", "archives"),
        ("application/octet-stream", "notes.md", "documents"),
        // The type is nothing and the extension is everything, which is most of what arrives.
        ("application/octet-stream", "costs.csv", "spreadsheets"),
        ("application/octet-stream", "backup.tar.gz", "archives"),
        ("application/octet-stream", "firmware.bin", "other"),
    ] {
        assert_eq!(
            super::category_of(mime_type, filename),
            expected,
            "{filename} as {mime_type}"
        );
    }
    // A charset on the header does not make it a different type.
    assert_eq!(
        super::category_of("text/calendar; charset=utf-8", "invite"),
        "invites"
    );
}

#[test]
fn the_library_lists_every_file_newest_first_and_filters_by_type_and_sender() {
    let conn = open();
    with_file(&conn, &pdf("pdf", "maya@example.org", "Quote", 300));
    with_file(&conn, &image("img", "ana@example.org", "Plans", 200, "plan.png", 400_000, false));
    with_file(
        &conn,
        &File {
            id: "csv",
            address: "maya@example.org",
            subject: "Costs",
            at: 100,
            filename: "costs.csv",
            mime_type: "text/csv",
            size: 2_000,
            inline: false,
        },
    );

    let all = super::files(&conn, "", "").expect("the library");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].attachment.filename, "quote.pdf");
    assert_eq!(all[0].category, "pdfs");
    assert_eq!(all[0].sender.address, "maya@example.org");
    assert_eq!(all[0].subject, "Quote");
    assert_eq!(all[2].attachment.filename, "costs.csv");

    let images = super::files(&conn, "images", "").expect("the images");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].attachment.filename, "plan.png");

    let hers = super::files(&conn, "", "MAYA@example.org").expect("one sender");
    assert_eq!(hers.len(), 2);
    assert!(hers.iter().all(|card| card.sender.address == "maya@example.org"));
}

#[test]
fn a_signature_image_is_not_in_the_library() {
    let conn = open();
    with_file(&conn, &image("sig", "news@brand.example", "The week", 300, "logo.png", 6_144, true));
    with_file(&conn, &image("real", "ana@example.org", "Plans", 200, "plan.png", 400_000, false));
    // Inline and an image, but far too big to be a signature: an embedded photograph is a file.
    with_file(&conn, &image("big", "ana@example.org", "Photo", 100, "photo.png", 60_000, true));

    let images = super::files(&conn, "images", "").expect("the images");
    let names: Vec<&str> = images
        .iter()
        .map(|card| card.attachment.filename.as_str())
        .collect();
    assert_eq!(names, vec!["plan.png", "photo.png"]);
}

#[test]
fn the_library_leaves_out_trash_and_spam() {
    let conn = open();
    let key = with_file(&conn, &pdf("gone", "maya@example.org", "Quote", 300));
    conn.execute("UPDATE threads SET trashed = 1 WHERE thread_key = ?1", [&key])
        .expect("trashed");

    assert!(super::files(&conn, "", "").expect("the library").is_empty());
}

#[test]
fn a_file_on_a_merged_thread_opens_the_thread_it_shows_under() {
    let conn = open();
    let merged = with_file(&conn, &pdf("one", "maya@example.org", "Quote", 300));
    let source = with_file(
        &conn,
        &image("two", "ana@example.org", "Re: quote", 200, "plan.png", 400_000, false),
    );
    state::write::merge_threads(&conn, std::slice::from_ref(&source), &merged).expect("a merge");

    let cards = super::files(&conn, "", "").expect("the library");
    assert_eq!(cards.len(), 2);
    assert!(
        cards.iter().all(|card| card.thread_key == merged),
        "a card opens the thread the list shows, not the one the message arrived in"
    );
}
