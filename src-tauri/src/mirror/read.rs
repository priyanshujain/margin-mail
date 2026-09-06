// The view: the mirror joined to the state database and handed over already ordered and already
// grouped.
//
// One SQL statement per call, across both databases on the one connection `db` opens. The
// grouping is part of the view, so it is decided here and never again: computing it a second time
// in TypeScript is how the two languages come to disagree about what "new" means.
//
// Every place is here. The routed three are a lookup in `state.sender_rules`, the piles are
// membership of `state.piles` and Snoozed is `state.snoozes` minus whatever has already come back;
// all four of those tables are the state database's, which is why a view is one statement across
// both files rather than two queries joined in Rust.

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OptionalExtension};

use crate::dto::{
    Destination,
    Attachment, LabelInfo, MergedSource, MessageView, Note, Person, Pile, Place, StorageUsed,
    Surface, ThreadPage, ThreadQuery, ThreadSummary, ThreadView, Unsubscribe,
};

use crate::routing;
use crate::state;

use super::fts;
use super::write;

const MAX_LIMIT: u32 = 200;

/// A cursor is the position in the order the list is already in: the group, the moment, and the
/// provider's thread id to break a tie. Opaque to the frontend, and keyed rather than an offset,
/// because mail arrives at the top of the list while somebody is reading the bottom of it.
struct Cursor {
    rank: i64,
    latest_ms: i64,
    tid: String,
}

impl Cursor {
    fn parse(raw: Option<&String>) -> Cursor {
        let none = Cursor {
            rank: -1,
            latest_ms: 0,
            tid: String::new(),
        };
        let Some(raw) = raw else { return none };
        let mut parts = raw.splitn(3, '|');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(rank), Some(ms), Some(tid)) => match (rank.parse(), ms.parse()) {
                (Ok(rank), Ok(latest_ms)) => Cursor {
                    rank,
                    latest_ms,
                    tid: tid.to_string(),
                },
                _ => none,
            },
            _ => none,
        }
    }

    fn of(rank: i64, latest_ms: i64, tid: &str) -> String {
        format!("{rank}|{latest_ms}|{tid}")
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

/// The group a row sits under, everywhere but the Inbox. Age is the only thing the other places
/// have to group by, and the buckets fall in the same order as the sort, so a keyed cursor over
/// both stays consistent.
/// By age, with Back above them.
///
/// A returned snooze goes to the top of whichever place the thread lives in, not only the Inbox,
/// which is what features.md section 7 asks for. The rank leaves 0 free for it in both grouping
/// expressions, so `group_rank` means the same thing whichever place asked.
fn age_groups(now: i64, params: &mut Vec<Value>) -> (String, String) {
    let today = now - write::DAY_MS;
    let week = now - 7 * write::DAY_MS;
    let month = now - 30 * write::DAY_MS;
    for _ in 0..2 {
        params.push(Value::Integer(today));
        params.push(Value::Integer(week));
        params.push(Value::Integer(month));
    }
    (
        "CASE WHEN rt.thread_key IS NOT NULL THEN 'back' \
         WHEN t.latest_ms >= ? THEN 'today' WHEN t.latest_ms >= ? THEN 'this-week' \
         WHEN t.latest_ms >= ? THEN 'this-month' ELSE 'earlier' END"
            .to_string(),
        "CASE WHEN rt.thread_key IS NOT NULL THEN 0 \
         WHEN t.latest_ms >= ? THEN 1 WHEN t.latest_ms >= ? THEN 2 \
         WHEN t.latest_ms >= ? THEN 3 ELSE 4 END"
            .to_string(),
    )
}

/// The Inbox: one list in time order, with Back above it for a snooze that has returned and is
/// waiting to be opened.
///
/// The group still says whether a row is new, because the list draws a new row heavier and the
/// badge counts them, and an ignored thread is never new: it still receives its messages and still
/// appends them, which is what ignoring means and not muting, but it does not read as new to you.
/// That is decided here rather than in the frontend, because a second opinion about it there is
/// how the two come to disagree. New and seen share a rank, so they interleave by time. There used
/// to be a New for you group over a Previously seen one, and three signals for one bit (a band, a
/// dot and a weight) read as a bug whenever one of them lagged; Superhuman's one list with one
/// weight is the shape that cannot.
fn inbox_groups() -> (String, String) {
    (
        "CASE WHEN rt.thread_key IS NOT NULL THEN 'back' \
         WHEN t.unseen = 1 AND COALESCE(tf.ignored, 0) = 0 THEN 'new' ELSE 'seen' END"
            .to_string(),
        "CASE WHEN rt.thread_key IS NOT NULL THEN 0 ELSE 1 END".to_string(),
    )
}

/// What each place is, as a predicate over the joined row.
fn place_clause(
    conn: &Connection,
    place: Place,
    label_id: Option<&str>,
    params: &mut Vec<Value>,
) -> Result<String, String> {
    Ok(match place {
        // A snoozed thread is away until it comes back, and a piled thread is in its pile.
        //
        // The routed half is two branches and only two: the sender's rule says Inbox, or there is
        // no rule at all and this account has written in the thread. The second is the override
        // from features.md section 2, and it is scoped exactly that narrowly on purpose. A reply
        // to a thread whose sender belongs in the Feed stays in the Feed, because that is where
        // that thread is; it is only a thread nobody has decided about that a reply pulls in here.
        Place::Inbox => {
            let mut routed = routing::rule_is(Destination::Inbox);
            if !routing::holds_replies(conn) {
                routed = format!(
                    "({routed} OR (NOT {} AND {}))",
                    routing::HAS_RULE, routing::IN_THREAD
                );
            }
            format!(
                "t.in_inbox = 1 AND t.trashed = 0 AND t.spam = 0 AND {routed} \
                 AND pl.thread_key IS NULL \
                 AND (sn.thread_key IS NULL OR rt.thread_key IS NOT NULL)"
            )
        }
        // Everything is everything the device holds, spam included. A message Gmail junked is
        // still yours and is already here with its body indexed, and a place that filtered it out
        // would be one more list you have to already know the answer to search. Trash stays out:
        // a thread you threw away is not one you are looking through.
        Place::Everything => "t.trashed = 0".to_string(),
        Place::Sent => "t.trashed = 0 AND EXISTS (SELECT 1 FROM messages m \
                        WHERE m.provider_thread_id = t.provider_thread_id AND m.sent = 1)"
            .to_string(),
        Place::Drafts => "t.has_draft = 1 AND t.trashed = 0".to_string(),
        Place::Starred => "t.starred = 1 AND t.trashed = 0".to_string(),
        Place::Spam => "t.spam = 1".to_string(),
        Place::Trash => "t.trashed = 1".to_string(),
        Place::Label => {
            let label = label_id.ok_or("that place needs a label")?;
            params.push(text(label));
            params.push(text(&label.to_lowercase()));
            "t.trashed = 0 AND t.spam = 0 AND EXISTS (\
                SELECT 1 FROM messages m JOIN json_each(m.labels) je \
                WHERE m.provider_thread_id = t.provider_thread_id \
                  AND (je.value = ? OR je.value IN (SELECT id FROM labels WHERE lower(name) = ?)))"
                .to_string()
        }
        // The three routed places are a sender rule lookup, and the loose conditions are the same
        // as the Inbox's: a thread in a pile is in its pile and a snoozed thread is away.
        Place::Feed | Place::PaperTrail | Place::ScreenedOut => {
            let destination = match place {
                Place::Feed => Destination::Feed,
                Place::PaperTrail => Destination::PaperTrail,
                _ => Destination::ScreenedOut,
            };
            let loose = if place == Place::ScreenedOut {
                // Screened out is a place you go looking, not a stream, so a pile or a snooze does
                // not take a thread out of it.
                ""
            } else {
                " AND pl.thread_key IS NULL \
                  AND (sn.thread_key IS NULL OR rt.thread_key IS NOT NULL)"
            };
            format!(
                "t.trashed = 0 AND t.spam = 0 AND {}{loose}",
                routing::rule_is(destination)
            )
        }
        // The Screener is the absence of a decision rather than a decision, which is why it is not
        // a lookup: a sender with no rule at all, and neither override applying. Expressed here
        // rather than filtered in Rust because the Inbox carries a pill counting the senders
        // waiting and that count must not load them.
        Place::Screener => {
            let mut clause = format!(
                "t.trashed = 0 AND t.spam = 0 AND t.from_address <> '' AND NOT {}",
                routing::HAS_RULE
            );
            if !routing::holds_replies(conn) {
                clause.push_str(&format!(" AND NOT {}", routing::IN_THREAD));
            }
            clause
        }
        // The piles are membership of `state.piles` and nothing else. A piled thread is out of
        // every other list by the loose conditions above, so this is the only place it shows, and
        // trash and spam are the only two things that take it out of its own pile.
        Place::ReplyLater | Place::SetAside => {
            let name = if place == Place::ReplyLater {
                "reply-later"
            } else {
                "set-aside"
            };
            params.push(text(name));
            "t.trashed = 0 AND t.spam = 0 AND pl.pile = ?".to_string()
        }
        // Waiting to return, and not one that already has. A snooze that has come back is spent and
        // its row is gone, so the second half is for a device that returned a thread and has not
        // finished writing it away.
        Place::Snoozed => "t.trashed = 0 AND t.spam = 0 \
                           AND sn.thread_key IS NOT NULL AND rt.thread_key IS NULL"
            .to_string(),
        Place::Search => return Err("a search is built from its query, not from its place".into()),
    })
}

/// The search predicate: an FTS5 match over the index, and the operators that are columns rather
/// than words as ordinary SQL beside it.
fn search_clause(query: &fts::Query, params: &mut Vec<Value>) -> String {
    let mut clauses = vec!["1 = 1".to_string()];
    if let Some(expr) = query.match_expr() {
        params.push(text(&expr));
        clauses.push("m.id IN (SELECT message_id FROM search WHERE search MATCH ?)".to_string());
    }
    if query.has_attachment {
        clauses.push("m.has_attachment = 1".to_string());
    }
    if let Some(before) = query.before_ms {
        params.push(Value::Integer(before));
        clauses.push("m.date_ms < ?".to_string());
    }
    if let Some(after) = query.after_ms {
        params.push(Value::Integer(after));
        clauses.push("m.date_ms >= ?".to_string());
    }
    for label in &query.label {
        params.push(text(label));
        params.push(text(&label.to_lowercase()));
        clauses.push(
            "EXISTS (SELECT 1 FROM json_each(m.labels) je \
             WHERE je.value = ? OR je.value IN (SELECT id FROM labels WHERE lower(name) = ?))"
                .to_string(),
        );
    }
    format!(
        "t.provider_thread_id IN (SELECT m.provider_thread_id FROM messages m WHERE {})",
        clauses.join(" AND ")
    )
}

/// The order the groups come in, for the one case a list is not a single statement: several
/// accounts merged into one view. The names and their order are decided in SQL above and repeated
/// here rather than derived, because there are only a few of them and a merge sort needs a number.
/// New and seen share one on purpose: the Inbox is a single list in time order under Back.
pub fn group_rank(group: &str) -> u8 {
    match group {
        "back" => 0,
        "new" | "seen" | "today" => 1,
        "this-week" => 2,
        "this-month" => 3,
        _ => 4,
    }
}

/// `in:` names a place, and an unknown one is not a place at all rather than every place.
pub fn place_named(name: &str) -> Option<Place> {
    Some(match name {
        "inbox" => Place::Inbox,
        "feed" => Place::Feed,
        "paper-trail" | "papertrail" => Place::PaperTrail,
        "everything" | "all" | "anywhere" => Place::Everything,
        "sent" => Place::Sent,
        "drafts" | "draft" => Place::Drafts,
        "starred" => Place::Starred,
        "screened-out" | "screenedout" => Place::ScreenedOut,
        "spam" => Place::Spam,
        "trash" => Place::Trash,
        _ => return None,
    })
}

const COLUMNS: &str = "\
    t.thread_key AS key,
    t.provider_thread_id AS tid,
    t.latest_ms AS latest_ms,
    COALESCE(rn.name, t.subject) AS subject,
    CASE WHEN rn.name IS NULL THEN NULL ELSE t.subject END AS original_subject,
    t.from_name AS from_name,
    t.from_address AS from_address,
    t.participants AS participants,
    t.snippet AS snippet,
    t.message_count AS message_count,
    t.unseen AS unseen,
    t.starred AS starred,
    t.trashed AS trashed,
    t.spam AS spam,
    t.has_attachment AS has_attachment,
    t.has_draft AS has_draft,
    pl.pile AS pile,
    COALESCE(sn.return_at, rt.due_ms) AS snoozed_until,
    COALESCE(tf.ignored, 0) AS ignored,
    COALESCE(tf.notify, 0) AS notify,
    EXISTS (SELECT 1 FROM chain WHERE chain.key = t.thread_key) AS merged,
    (SELECT n.body FROM state.notes n WHERE n.thread_key = t.thread_key AND n.deleted = 0
      ORDER BY n.created_at DESC, n.id DESC LIMIT 1) AS note,
    EXISTS (SELECT 1 FROM outbox o WHERE o.op = 'send' AND o.thread_key = t.thread_key) AS sending";

const JOINS: &str = "\
    FROM threads t
    LEFT JOIN state.piles pl ON pl.thread_key = t.thread_key
    LEFT JOIN state.snoozes sn ON sn.thread_key = t.thread_key
    LEFT JOIN state.renames rn ON rn.thread_key = t.thread_key
    LEFT JOIN state.thread_flags tf ON tf.thread_key = t.thread_key
    LEFT JOIN state.returned rt ON rt.thread_key = t.thread_key";

/// A thread merged into another shows under that other one's key and never under its own. Named
/// rather than written twice because the badge counts the same rows the list draws, and a row that
/// is a row to one of them and not to the other is the bug this whole arrangement is avoiding.
const NOT_MERGED_AWAY: &str =
    "NOT EXISTS (SELECT 1 FROM state.merges gone WHERE gone.thread_key = t.thread_key)";

/// One page of one place, for one account. Ordered, grouped, and paged on a key rather than an
/// offset. The whole view is this statement; there is no second pass in Rust.
pub fn threads_list(
    conn: &Connection,
    account_id: &str,
    account_color: &str,
    query: &ThreadQuery,
    now: i64,
) -> Result<ThreadPage, String> {
    let limit = query.limit.clamp(1, MAX_LIMIT);
    let mut params: Vec<Value> = Vec::new();

    let (group_expr, rank_expr) = match query.place {
        Place::Inbox => inbox_groups(),
        _ => age_groups(now, &mut params),
    };

    let parsed = query.query.as_deref().map(fts::parse);
    let clause = match query.place {
        Place::Search => {
            let parsed = parsed.as_ref().ok_or("a search needs a query")?;
            let mut clause = search_clause(parsed, &mut params);
            // No place filter of its own. The message you most need to find is the one something
            // else decided you should not see, so a search reaches trash, spam and screened out
            // like anything else on the device; `in:` is how you narrow it back down.
            if let Some(place) = parsed.place.as_deref().and_then(place_named) {
                clause = format!(
                    "{clause} AND ({})",
                    place_clause(conn, place, query.label_id.as_deref(), &mut params)?
                );
            }
            clause
        }
        place => place_clause(conn, place, query.label_id.as_deref(), &mut params)?,
    };

    let cursor = Cursor::parse(query.cursor.as_ref());
    for _ in 0..3 {
        params.push(Value::Integer(cursor.rank));
    }
    params.push(Value::Integer(cursor.latest_ms));
    params.push(Value::Integer(cursor.latest_ms));
    params.push(text(&cursor.tid));
    params.push(Value::Integer(limit as i64));

    // `state::read::CHAIN` is the one definition of what a merge chain is, so the statement borrows
    // it rather than spelling a second one out here that could drift from it.
    let chain = state::read::CHAIN;
    let sql = format!(
        "{chain}, page AS (
           SELECT {COLUMNS},
                  {group_expr} AS grp,
                  {rank_expr} AS grp_rank
           {JOINS}
           WHERE {NOT_MERGED_AWAY}
             AND ({clause})
         )
         SELECT * FROM page
         WHERE ? < 0
            OR grp_rank > ?
            OR (grp_rank = ? AND (latest_ms < ? OR (latest_ms = ? AND tid > ?)))
         ORDER BY grp_rank ASC, latest_ms DESC, tid ASC
         LIMIT ?"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(params), |row| {
            Ok((
                ThreadSummary {
                    key: row.get("key")?,
                    account_id: account_id.to_string(),
                    account_color: account_color.to_string(),
                    subject: row.get("subject")?,
                    original_subject: row.get("original_subject")?,
                    from: Person {
                        name: row.get("from_name")?,
                        address: row.get("from_address")?,
                    },
                    participants: serde_json::from_str(&row.get::<_, String>("participants")?)
                        .unwrap_or_default(),
                    snippet: row.get("snippet")?,
                    date_ms: row.get("latest_ms")?,
                    message_count: row.get::<_, i64>("message_count")? as u32,
                    unseen: row.get::<_, i64>("unseen")? != 0,
                    starred: row.get::<_, i64>("starred")? != 0,
                    trashed: row.get::<_, i64>("trashed")? != 0,
                    spam: row.get::<_, i64>("spam")? != 0,
                    has_attachment: row.get::<_, i64>("has_attachment")? != 0,
                    has_draft: row.get::<_, i64>("has_draft")? != 0,
                    pile: row
                        .get::<_, Option<String>>("pile")?
                        .and_then(|pile| match pile.as_str() {
                            "reply-later" => Some(Pile::ReplyLater),
                            "set-aside" => Some(Pile::SetAside),
                            _ => None,
                        }),
                    snoozed_until: row.get("snoozed_until")?,
                    ignored: row.get::<_, i64>("ignored")? != 0,
                    notify: row.get::<_, i64>("notify")? != 0,
                    merged: row.get::<_, i64>("merged")? != 0,
                    note: row.get("note")?,
                    group: row.get("grp")?,
                    sending: row.get::<_, i64>("sending")? != 0,
                },
                row.get::<_, i64>("grp_rank")?,
                row.get::<_, String>("tid")?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let next_cursor = (rows.len() as u32 == limit)
        .then(|| {
            rows.last()
                .map(|(summary, rank, tid)| Cursor::of(*rank, summary.date_ms, tid))
        })
        .flatten();

    Ok(ThreadPage {
        threads: rows.into_iter().map(|(summary, _, _)| summary).collect(),
        next_cursor,
        footer: footer(conn, query.place)?,
    })
}

/// How many unseen threads this account has in the Inbox, ignored ones left out. One account's
/// half of the number on the dock.
///
/// Built from `place_clause` and `inbox_groups`, which is the point of it being here rather than in
/// the badge package: a badge counting something other than what the Inbox shows is worse than no
/// badge, and the only way that cannot happen is for there to be one definition of "new in the
/// Inbox" rather than two. So this is `threads_list`'s statement with the columns taken off and the
/// group pinned, which is an index scan instead of a page of snippets and notes nobody reads.
///
/// Not the mailbox's unread count. A thread held in the Screener, routed to the Feed or the Paper
/// Trail, piled, snoozed or ignored is unread and is not waiting for you, and the whole reason this
/// app exists is that the two numbers are not the same one.
pub fn inbox_unseen(conn: &Connection) -> Result<i64, String> {
    let mut params: Vec<Value> = Vec::new();
    let clause = place_clause(conn, Place::Inbox, None, &mut params)?;
    // Unseen and not ignored, which is the second arm of `inbox_groups` with the group itself left
    // out on purpose. New and Back are the same fact about a thread seen from either end:
    // Back only wins the label because a returned snooze is worth putting at the top, and a snooze
    // that came back is the plainest case there is of something waiting to be dealt with. Counting
    // the group would mean the badge did not move when tomorrow arrived, which is the one moment
    // the person asked to be reminded.
    //
    // Reading is still what clears it, because a thread in Back you have already read is not
    // unseen. A badge counting the whole of Back would be a number no amount of reading takes off.
    let sql = format!(
        "SELECT COUNT(*)
         {JOINS}
         WHERE {NOT_MERGED_AWAY}
           AND ({clause})
           AND t.unseen = 1
           AND COALESCE(tf.ignored, 0) = 0"
    );
    conn.query_row(&sql, params_from_iter(params), |row| row.get(0))
        .map_err(|e| e.to_string())
}

/// The window is visible in two places and nowhere else, and this is the first of them.
fn footer(conn: &Connection, place: Place) -> Result<Option<String>, String> {
    // Trash and Spam empty themselves on Gmail's clock, which is a second retention rule and the
    // only one the storage window does not explain. There is no Empty button to put here: deleting
    // for good needs the scope that is the whole mailbox, so the line states what will happen
    // anyway. Screened out gets none, because it falls off with the window like everything else.
    if matches!(place, Place::Trash | Place::Spam) {
        return Ok(Some("Gmail empties this after 30 days.".to_string()));
    }
    if place != Place::Everything {
        return Ok(None);
    }
    let days = write::window_days(conn)?;
    let span = match days {
        0 => return Ok(None),
        30 => "the last month".to_string(),
        90 => "the last three months".to_string(),
        180 => "the last six months".to_string(),
        365 => "the last year".to_string(),
        other => format!("the last {other} days"),
    };
    Ok(Some(format!(
        "Showing {span}. Older mail is on Gmail."
    )))
}

const VIEW_SQL: &str = "\
    -- The merge chain, because two of the predicates below resolve one and a merge of a merge is
    -- a chain. `state::read::CHAIN` is the same definition; it cannot be interpolated into a const.
    WITH RECURSIVE chain(source, key) AS (
        SELECT thread_key, merged_key FROM state.merges
        UNION
        SELECT c.source, m.merged_key FROM chain c JOIN state.merges m ON m.thread_key = c.key
    )\
    SELECT
        m.id AS id,
        COALESCE(m.message_id, '') AS message_id,
        m.thread_key AS thread_key,
        m.from_name AS from_name,
        m.from_address AS from_address,
        m.to_json AS to_json,
        m.cc_json AS cc_json,
        m.bcc_json AS bcc_json,
        m.reply_to_json AS reply_to_json,
        m.date_ms AS date_ms,
        m.subject AS subject,
        m.seen AS seen,
        m.draft AS draft,
        m.sent AS sent,
        m.labels AS labels,
        m.list_id AS list_id,
        m.list_unsubscribe AS list_unsubscribe,
        m.list_unsub_post AS list_unsub_post,
        COALESCE(b.html, '') AS html,
        -- No row in `bodies` at all, which is a message whose body has not been fetched yet
        -- rather than one that arrived empty. The two look identical in `html` and the reading
        -- pane has to tell them apart.
        b.message_id IS NULL AS body_pending,
        b.quoted_html AS quoted_html,
        COALESCE(b.is_html, 0) AS is_html,
        COALESCE(b.surface, 'theme') AS surface,
        COALESCE(b.trackers, '[]') AS trackers,
        COALESCE(b.blocked_images, 0) AS blocked_images,
        (SELECT json_group_array(json_object(
             'id', a.id, 'messageId', a.message_id, 'filename', a.filename,
             'mimeType', a.mime_type, 'size', a.size, 'inline', a.inline = 1,
             'contentId', a.content_id, 'cached', a.cached_path IS NOT NULL))
         FROM attachments a WHERE a.message_id = m.id) AS attachments_json,
        (SELECT name FROM state.renames WHERE thread_key = ?) AS rename,
        (SELECT subject FROM threads WHERE thread_key = ? ORDER BY latest_ms DESC LIMIT 1)
            AS thread_subject,
        (SELECT pile FROM state.piles WHERE thread_key = ?) AS pile,
        (SELECT return_at FROM state.snoozes WHERE thread_key = ?) AS snoozed_until,
        COALESCE((SELECT ignored FROM state.thread_flags WHERE thread_key = ?), 0) AS ignored,
        COALESCE((SELECT notify FROM state.thread_flags WHERE thread_key = ?), 0) AS notify,
        COALESCE((SELECT MAX(starred) FROM threads WHERE thread_key = ?), 0) AS starred,
        COALESCE((SELECT MAX(trashed) FROM threads WHERE thread_key = ?), 0) AS trashed,
        COALESCE((SELECT MAX(spam) FROM threads WHERE thread_key = ?), 0) AS spam,
        (SELECT json_group_array(json_object(
             'id', n.id, 'threadKey', n.thread_key, 'body', n.body,
             'createdAtMs', n.created_at, 'afterMessageId', n.after_message_id))
         FROM state.notes n WHERE n.thread_key = ? AND n.deleted = 0) AS notes_json,
        (SELECT json_group_array(json_object(
             'key', mg.thread_key, 'subject', COALESCE(th.subject, '')))
         FROM state.merges mg LEFT JOIN threads th ON th.thread_key = mg.thread_key
         WHERE mg.merged_key = ?) AS merged_json
    FROM messages m
    LEFT JOIN bodies b ON b.message_id = m.id
    WHERE m.hydrated = 1 AND m.provider_thread_id IN (
        SELECT provider_thread_id FROM threads
        WHERE thread_key = ?
           OR thread_key IN (SELECT source FROM chain WHERE key = ?)
    )
    ORDER BY m.date_ms ASC, m.id ASC";

/// The whole thread, for the reading pane. One statement again: the thread's own state repeats on
/// every message row, which costs a few bytes and saves a second round trip and a second truth.
pub fn thread_view(
    conn: &Connection,
    account_id: &str,
    key: &str,
    own_addresses: &[String],
) -> Result<ThreadView, String> {
    let mut stmt = conn.prepare(VIEW_SQL).map_err(|e| e.to_string())?;
    let keys: Vec<Value> = (0..13).map(|_| text(key)).collect();
    let rows = stmt
        .query_map(params_from_iter(keys), |row| {
            Ok((
                MessageView {
                    id: row.get("id")?,
                    message_id: row.get("message_id")?,
                    thread_key: row.get("thread_key")?,
                    from: Person {
                        name: row.get("from_name")?,
                        address: row.get("from_address")?,
                    },
                    to: parse_people(row.get::<_, String>("to_json")?),
                    cc: parse_people(row.get::<_, String>("cc_json")?),
                    bcc: parse_people(row.get::<_, String>("bcc_json")?),
                    reply_to: parse_people(row.get::<_, String>("reply_to_json")?),
                    date_ms: row.get("date_ms")?,
                    subject: row.get("subject")?,
                    html: row.get("html")?,
                    body_pending: row.get::<_, i64>("body_pending")? != 0,
                    quoted_html: row.get("quoted_html")?,
                    is_html: row.get::<_, i64>("is_html")? != 0,
                    // Anything the column does not recognise is the theme, which is the app's own
                    // paper and so the answer that can never be unreadable.
                    surface: match row.get::<_, String>("surface")?.as_str() {
                        "paper" => Surface::Paper,
                        _ => Surface::Theme,
                    },
                    attachments: serde_json::from_str(
                        &row.get::<_, Option<String>>("attachments_json")?
                            .unwrap_or_else(|| "[]".to_string()),
                    )
                    .unwrap_or_default(),
                    trackers: serde_json::from_str(&row.get::<_, String>("trackers")?)
                        .unwrap_or_default(),
                    blocked_images: row.get::<_, i64>("blocked_images")? as u32,
                    // The sanitiser fetches nothing, so a body is only ever served with its remote
                    // images already inlined by the caller that asked for them.
                    images_loaded: false,
                    seen: row.get::<_, i64>("seen")? != 0,
                    draft: row.get::<_, i64>("draft")? != 0,
                    sent_by_me: row.get::<_, i64>("sent")? != 0
                        || own_addresses.contains(&row.get::<_, String>("from_address")?),
                    // The invite is a product of the render and the frozen `bodies` table has
                    // nowhere to keep one, so it arrives with the message pipeline's own schema
                    // step rather than being half-stored here.
                    invite: None,
                    unsubscribe: unsubscribe(
                        row.get::<_, Option<String>>("list_unsubscribe")?.as_deref(),
                        row.get::<_, Option<String>>("list_unsub_post")?.as_deref(),
                    ),
                    list_id: row.get("list_id")?,
                },
                row.get::<_, String>("labels")?,
                ThreadState {
                    rename: row.get("rename")?,
                    subject: row.get::<_, Option<String>>("thread_subject")?.unwrap_or_default(),
                    pile: row.get("pile")?,
                    snoozed_until: row.get("snoozed_until")?,
                    ignored: row.get::<_, i64>("ignored")? != 0,
                    notify: row.get::<_, i64>("notify")? != 0,
                    starred: row.get::<_, i64>("starred")? != 0,
                    trashed: row.get::<_, i64>("trashed")? != 0,
                    spam: row.get::<_, i64>("spam")? != 0,
                    notes: row.get::<_, Option<String>>("notes_json")?,
                    merged: row.get::<_, Option<String>>("merged_json")?,
                },
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Err("that thread is not on this device".to_string());
    }

    let state = rows.last().map(|(_, _, state)| state.clone()).expect("a row");
    let mut participants: Vec<Person> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut messages = Vec::new();
    for (message, message_labels, _) in rows {
        for label in serde_json::from_str::<Vec<String>>(&message_labels).unwrap_or_default() {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        for person in std::iter::once(message.from.clone()).chain(message.to.iter().cloned()) {
            if !person.address.is_empty()
                && !participants.iter().any(|held| held.address == person.address)
            {
                participants.push(person);
            }
        }
        messages.push(message);
    }

    Ok(ThreadView {
        key: key.to_string(),
        account_id: account_id.to_string(),
        subject: state.rename.clone().unwrap_or_else(|| state.subject.clone()),
        original_subject: state.rename.as_ref().map(|_| state.subject.clone()),
        participants,
        messages,
        notes: state
            .notes
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<Note>>(json).ok())
            .unwrap_or_default(),
        merged_from: state
            .merged
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<MergedSource>>(json).ok())
            .unwrap_or_default(),
        pile: state.pile.as_deref().and_then(|pile| match pile {
            "reply-later" => Some(Pile::ReplyLater),
            "set-aside" => Some(Pile::SetAside),
            _ => None,
        }),
        snoozed_until: state.snoozed_until,
        ignored: state.ignored,
        notify: state.notify,
        starred: state.starred,
        trashed: state.trashed,
        spam: state.spam,
        labels,
    })
}

#[derive(Clone)]
struct ThreadState {
    rename: Option<String>,
    subject: String,
    pile: Option<String>,
    snoozed_until: Option<i64>,
    ignored: bool,
    notify: bool,
    starred: bool,
    trashed: bool,
    spam: bool,
    notes: Option<String>,
    merged: Option<String>,
}

fn parse_people(json: String) -> Vec<Person> {
    serde_json::from_str(&json).unwrap_or_default()
}

/// `List-Unsubscribe` in the three forms it arrives in. RFC 8058's one-click is the header pair,
/// and without the `POST` half the URL is a page to visit rather than a button to press.
///
/// Public because the `unsubscribe` command has to choose between the same three, and two readings
/// of one header is how a button comes to promise something the pane did not offer.
pub fn unsubscribe(header: Option<&str>, post: Option<&str>) -> Option<Unsubscribe> {
    let header = header?;
    let mut mailto = None;
    let mut url = None;
    for part in header.split(',') {
        let value = part.trim().trim_start_matches('<').trim_end_matches('>').trim();
        if value.starts_with("mailto:") {
            mailto = Some(value.to_string());
        } else if value.starts_with("http") {
            url = Some(value.to_string());
        }
    }
    if mailto.is_none() && url.is_none() {
        return None;
    }
    Some(Unsubscribe {
        one_click: url.is_some()
            && post
                .map(|value| value.to_ascii_lowercase().contains("one-click"))
                .unwrap_or(false),
        mailto,
        url,
    })
}

/// Every message of a thread whose cached render was made by an older sanitiser, so the caller can
/// render them again before the pane asks for them.
pub fn stale_renders(conn: &Connection, key: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE chain(source, key) AS (
                 SELECT thread_key, merged_key FROM state.merges
                 UNION
                 SELECT c.source, m.merged_key FROM chain c JOIN state.merges m ON m.thread_key = c.key
             )
             SELECT b.message_id FROM bodies b
             JOIN messages m ON m.id = b.message_id
             WHERE b.raw IS NOT NULL AND b.render_version <> ?1
               AND m.provider_thread_id IN (
                   SELECT provider_thread_id FROM threads
                   WHERE thread_key = ?2
                      OR thread_key IN (SELECT source FROM chain WHERE key = ?2))",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![crate::mime::RENDER_VERSION, key],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The newest messages anywhere in the mirror whose cached render was made by an older sanitiser,
/// up to a limit, for the pass that renews them in the background. Newest first because those are
/// the threads about to be opened.
pub fn stale_renders_any(conn: &Connection, limit: usize) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT b.message_id FROM bodies b
             JOIN messages m ON m.id = b.message_id
             WHERE b.raw IS NOT NULL AND b.render_version <> ?1
             ORDER BY m.date_ms DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![crate::mime::RENDER_VERSION, limit as i64],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Whether this account is the one holding a thread, which is the question every command taking a
/// thread key has to answer before it can do anything else.
///
/// Two index seeks and a `LIMIT 1` rather than a count: an open asks this of every account in turn
/// and only wants to know that one of them said yes. The count it used to run read the whole
/// `threads` table on every account on every open, because a predicate of the shape
/// `thread_key = ? OR thread_key IN (subquery)` cannot use the index on `thread_key`.
pub fn holds_thread(conn: &Connection, key: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM threads WHERE thread_key = ?1
         UNION ALL
         SELECT 1 FROM threads
          WHERE thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1)
         LIMIT 1",
        [key],
        |_| Ok(()),
    )
    .optional()
    .map(|held| held.is_some())
    .map_err(|e| e.to_string())
}

/// The messages of a thread that opening it marks seen: hydrated, unseen, and not a draft. The
/// same three-part rule `write::refresh_thread` folds into `threads.unseen`, so a thread the list
/// already calls seen answers with nothing here and an open of it writes nothing.
pub fn unseen_message_ids(conn: &Connection, key: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT m.id FROM messages m
             WHERE m.hydrated = 1 AND m.seen = 0 AND m.draft = 0
               AND m.provider_thread_id IN (
                   SELECT provider_thread_id FROM threads
                   WHERE thread_key = ?1
                      OR thread_key IN (SELECT thread_key FROM state.merges WHERE merged_key = ?1))
             ORDER BY m.date_ms ASC, m.id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([key], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// The provider ids of a thread's messages that have no body yet, which is what an open fetches.
pub fn bodiless(conn: &Connection, key: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE chain(source, key) AS (
                 SELECT thread_key, merged_key FROM state.merges
                 UNION
                 SELECT c.source, m.merged_key FROM chain c JOIN state.merges m ON m.thread_key = c.key
             )
             SELECT m.id FROM messages m
             LEFT JOIN bodies b ON b.message_id = m.id
             WHERE b.message_id IS NULL AND m.hydrated = 1
               AND m.provider_thread_id IN (
                   SELECT provider_thread_id FROM threads
                   WHERE thread_key = ?1
                      OR thread_key IN (SELECT source FROM chain WHERE key = ?1))
             ORDER BY m.date_ms DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([key], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// A comma separated run of bound placeholders. Two queries below exclude a set whose size is not
/// known until runtime, and a set that has to interact with a `LIMIT` cannot be a loop of single
/// row statements the way the rest of this module does it.
fn holes(count: usize) -> String {
    std::iter::repeat_n("?", count).collect::<Vec<_>>().join(",")
}

/// The clause that leaves out what the caller has given up on, or nothing at all when it has not
/// given up on anything, which is the ordinary case and should prepare the ordinary statement.
fn excluding(skip: &[String]) -> String {
    if skip.is_empty() {
        String::new()
    } else {
        format!(" AND m.id NOT IN ({})", holes(skip.len()))
    }
}

/// The newest messages inside the window with no body, which is what the body cache works through.
///
/// `skip` is what the provider would not give up. Without it this returns the same head of the
/// queue on every pass: a message deleted upstream between the listing and the fetch answers 404
/// for ever, it sorts newest first like anything else, and a handful of them is a cache that never
/// reaches the mail behind them. The caller is what remembers, so this stays a query.
///
/// Transient rows are left out for a different reason. They came from a provider search rather
/// than from the window, and the next eviction pass takes them away again unless they gained a
/// pile or a note in the meantime, so a body fetched for one is 20 units spent on a message that
/// is already on its way out. A hundred of them landing at the head of the queue after one search
/// would be the same head of line stall by another route. `bodiless` deliberately does not do
/// this: somebody opening a search hit wants to read it, whatever is going to happen to the row.
pub fn prefetchable(
    conn: &Connection,
    limit: usize,
    skip: &[String],
) -> Result<Vec<String>, String> {
    let sql = format!(
        "SELECT m.id FROM messages m
         LEFT JOIN bodies b ON b.message_id = m.id
         WHERE b.message_id IS NULL AND m.hydrated = 1 AND m.transient = 0{}
         ORDER BY m.date_ms DESC LIMIT ?",
        excluding(skip)
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut args: Vec<Value> = skip.iter().map(|id| text(id)).collect();
    args.push(Value::Integer(limit as i64));
    let rows = stmt
        .query_map(params_from_iter(args), |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Which of these messages still have no body, which is how the cache forgets a message it once
/// could not fetch. Opening a thread asks for its bodies whatever the cache gave up on, so one
/// that somebody has since read is one the cache should stop skipping.
pub fn still_bodiless(conn: &Connection, ids: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for id in ids {
        let held: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM bodies WHERE message_id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if held.is_none() {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// How many messages inside the window the body cache could still fetch, which is what it counts
/// down and what the status bar divides by while it does.
///
/// The same two exclusions as `prefetchable`, and they have to be the same ones: work nobody is
/// going to do is not work outstanding, and counting it is a progress bar that stops short of the
/// end for ever.
pub fn prefetch_backlog(conn: &Connection, skip: &[String]) -> Result<u32, String> {
    let sql = format!(
        "SELECT COUNT(*) FROM messages m
         LEFT JOIN bodies b ON b.message_id = m.id
         WHERE b.message_id IS NULL AND m.hydrated = 1 AND m.transient = 0{}",
        excluding(skip)
    );
    let args: Vec<Value> = skip.iter().map(|id| text(id)).collect();
    conn.query_row(&sql, params_from_iter(args), |row| row.get::<_, i64>(0))
        .map(|count| count.max(0) as u32)
        .map_err(|e| e.to_string())
}

/// The account's own labels, for the palette and the label picker.
///
/// The provider's system labels are left out and the whole table is not. They are still stored,
/// because `in:` and `label:` resolve a name against it and the mirror writes `SPAM` and `TRASH`
/// on messages, but not one of them is worth a row: every one is a place this app already has
/// under a name of its own (Spam, Trash, Sent, Drafts, Starred), a flag with a key (`u`, `Shift+S`),
/// or a guess this app replaces with sender routing (Important, Gmail's category tabs). Offering
/// them means a palette with Spam in it twice, and a picker that will put `SPAM` on a thread as
/// though that were a label you chose. An IMAP account draws the same line: a folder the server
/// gave a role is one of these, and every other folder is the person's own.
pub fn labels(conn: &Connection, account_id: &str) -> Result<Vec<LabelInfo>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, kind FROM labels WHERE kind <> 'system' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LabelInfo {
                id: row.get(0)?,
                account_id: account_id.to_string(),
                name: row.get(1)?,
                kind: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn storage_used(conn: &Connection, account_id: &str) -> Result<StorageUsed, String> {
    let scalar = |sql: &str| -> Result<i64, String> {
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())
    };
    let page_size = scalar("PRAGMA page_size")?;
    Ok(StorageUsed {
        account_id: account_id.to_string(),
        messages: scalar("SELECT COUNT(*) FROM messages")? as u64,
        threads: scalar("SELECT COUNT(*) FROM threads")? as u64,
        mirror_bytes: (scalar("PRAGMA page_count")? * page_size) as u64,
        bodies_bytes: scalar(
            "SELECT COALESCE(SUM(LENGTH(COALESCE(raw, '')) + LENGTH(COALESCE(html, ''))), 0)
             FROM bodies",
        )? as u64,
        attachments_bytes: scalar(
            "SELECT COALESCE(SUM(size), 0) FROM attachments WHERE cached_path IS NOT NULL",
        )? as u64,
        state_bytes: (scalar("PRAGMA state.page_count")? * scalar("PRAGMA state.page_size")?)
            as u64,
        oldest_ms: conn
            .query_row(
                "SELECT MIN(date_ms) FROM messages WHERE hydrated = 1 AND date_ms > 0",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten(),
    })
}

pub fn thread_key_of(conn: &Connection, provider_thread_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT thread_key FROM threads WHERE provider_thread_id = ?1",
        [provider_thread_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// One message's view, taken out of its thread's rather than assembled a second time. Sharing the
/// statement is what keeps a message looking the same whichever command handed it over.
pub fn message_view(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
    own_addresses: &[String],
) -> Result<MessageView, String> {
    let key: Option<String> = conn
        .query_row(
            "SELECT thread_key FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let key = key.ok_or("that message is not on this device")?;
    thread_view(conn, account_id, &key, own_addresses)?
        .messages
        .into_iter()
        .find(|message| message.id == message_id)
        .ok_or_else(|| "that message is not on this device".to_string())
}

/// The bytes a message arrived as, which is what a second render works from.
pub fn raw_body(conn: &Connection, message_id: &str) -> Result<Option<Vec<u8>>, String> {
    conn.query_row(
        "SELECT raw FROM bodies WHERE message_id = ?1 AND raw IS NOT NULL",
        [message_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// One attachment as the cache needs it: what to call it, how big it is, which part of which
/// message it is, and where the bytes already are.
#[derive(Debug, Clone)]
pub struct AttachmentRow {
    pub id: String,
    pub message_id: String,
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub cached_path: Option<String>,
}

pub fn attachment_row(conn: &Connection, id: &str) -> Result<Option<AttachmentRow>, String> {
    conn.query_row(
        "SELECT id, message_id, COALESCE(part_id, ''), filename, mime_type, size, cached_path
         FROM attachments WHERE id = ?1",
        [id],
        |row| {
            Ok(AttachmentRow {
                id: row.get(0)?,
                message_id: row.get(1)?,
                part_id: row.get(2)?,
                filename: row.get(3)?,
                mime_type: row.get(4)?,
                size: row.get::<_, i64>(5)?.max(0) as u64,
                cached_path: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Everything the cache is holding, least recently fetched first, which is the order it gives
/// things up in.
pub fn cached_attachments(conn: &Connection) -> Result<Vec<(String, String, u64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, cached_path, size FROM attachments
             WHERE cached_path IS NOT NULL ORDER BY COALESCE(cached_at, 0) ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Attachment rows for one message, which the file cards and the forward path both want.
pub fn attachments(conn: &Connection, message_id: &str) -> Result<Vec<Attachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, message_id, filename, mime_type, size, inline, content_id,
                    cached_path IS NOT NULL
             FROM attachments WHERE message_id = ?1 ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([message_id], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size: row.get::<_, i64>(4)? as u64,
                inline: row.get::<_, i64>(5)? != 0,
                content_id: row.get(6)?,
                cached: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}
