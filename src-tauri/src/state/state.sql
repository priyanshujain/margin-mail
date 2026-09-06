-- The state database: everything the user decided. It is never derived, it is never sent to the
-- provider, and its keys are portable, so every decision reattaches when the same mail arrives
-- through another provider with the same `Message-ID`s.
--
-- Every table below is a materialised view of `journal`. Writing goes through the journal and then
-- into the table; replaying the journal from empty must reproduce the tables exactly, which is what
-- makes roaming through the backup store possible without a server. `state::journal::replay` is the
-- one function allowed to write these tables from anything but an event.
--
-- Merging two devices is last writer wins per key by `at_ms`, with the device id breaking a tie so
-- that two devices replaying the same pair of events in either order land in the same place. That
-- is correct for every kind of state here, because each one is a value somebody set rather than an
-- accumulation.

CREATE TABLE IF NOT EXISTS state.meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS state.journal (
    device_id TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    at_ms     INTEGER NOT NULL,
    kind      TEXT NOT NULL,
    key       TEXT NOT NULL,
    payload   TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (device_id, seq)
);

-- Exactly one destination per sender, keyed on the address or on the domain. Address rules beat
-- domain rules, which is a read-time decision rather than two tables.
CREATE TABLE IF NOT EXISTS state.sender_rules (
    subject     TEXT PRIMARY KEY,
    is_domain   INTEGER NOT NULL DEFAULT 0,
    destination TEXT NOT NULL,
    reason      TEXT,
    at_ms       INTEGER NOT NULL DEFAULT 0,
    device_id   TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS state.piles (
    thread_key TEXT PRIMARY KEY,
    pile       TEXT NOT NULL,
    -- Where in the stack. A pile is a stack of cards, so its order is part of the state.
    position   INTEGER NOT NULL DEFAULT 0,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    device_id  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS state.snoozes (
    thread_key TEXT PRIMARY KEY,
    return_at  INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    -- For `if-no-reply`: the latest message at the time, so a reply since can be recognised.
    watermark  INTEGER NOT NULL DEFAULT 0,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    device_id  TEXT NOT NULL DEFAULT ''
);

-- Returned and waiting in the Back group until the thread is opened.
--
-- The one table in this file that is not a view of the journal, and deliberately so: it has no
-- `device_id` and no tombstone because it is not meant to roam. Whichever device next opens the app
-- evaluates what is due, and a thread waiting in Back on the laptop is not a fact about the phone.
-- Here rather than in the mirror because the mirror can be rebuilt and a snooze that came back must
-- not come back twice.
CREATE TABLE IF NOT EXISTS state.returned (
    thread_key TEXT PRIMARY KEY,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    due_ms     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS state.notes (
    id                TEXT PRIMARY KEY,
    thread_key        TEXT NOT NULL,
    body              TEXT NOT NULL DEFAULT '',
    after_message_id  TEXT,
    created_at        INTEGER NOT NULL DEFAULT 0,
    at_ms             INTEGER NOT NULL DEFAULT 0,
    device_id         TEXT NOT NULL DEFAULT '',
    deleted           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS state.renames (
    thread_key TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    device_id  TEXT NOT NULL DEFAULT ''
);

-- A source thread key pointing at the key of the thread it now shows as part of. Unmerge deletes
-- the rows rather than writing an inverse, and the merged thread's own name lives in `renames`.
CREATE TABLE IF NOT EXISTS state.merges (
    thread_key TEXT PRIMARY KEY,
    merged_key TEXT NOT NULL,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    device_id  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS state.clips (
    id             TEXT PRIMARY KEY,
    thread_key     TEXT NOT NULL,
    message_id     TEXT NOT NULL,
    text           TEXT NOT NULL,
    sender_name    TEXT,
    sender_address TEXT NOT NULL DEFAULT '',
    subject        TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL DEFAULT 0,
    at_ms          INTEGER NOT NULL DEFAULT 0,
    device_id      TEXT NOT NULL DEFAULT '',
    deleted        INTEGER NOT NULL DEFAULT 0
);

-- The two per-thread switches. One row rather than two tables, because they are set on the same
-- thing by the same gesture and neither ever exists without the other being asked about.
CREATE TABLE IF NOT EXISTS state.thread_flags (
    thread_key TEXT PRIMARY KEY,
    ignored    INTEGER NOT NULL DEFAULT 0,
    notify     INTEGER NOT NULL DEFAULT 0,
    at_ms      INTEGER NOT NULL DEFAULT 0,
    device_id  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS state.contacts (
    address             TEXT PRIMARY KEY,
    note                TEXT,
    notify              INTEGER NOT NULL DEFAULT 0,
    allow_remote_images INTEGER NOT NULL DEFAULT 0,
    -- Trash this sender's Feed mail after so many days. NULL is never.
    auto_trash_days     INTEGER,
    bundle              INTEGER NOT NULL DEFAULT 1,
    at_ms               INTEGER NOT NULL DEFAULT 0,
    device_id           TEXT NOT NULL DEFAULT ''
);

-- The per-account preferences that should follow the person rather than the machine: the
-- signature, the instant intro text. Device preferences live in settings.json instead.
CREATE TABLE IF NOT EXISTS state.prefs (
    key       TEXT PRIMARY KEY,
    value     TEXT NOT NULL,
    at_ms     INTEGER NOT NULL DEFAULT 0,
    device_id TEXT NOT NULL DEFAULT ''
);

-- Where the Feed's "You left off here" line goes, per place.
CREATE TABLE IF NOT EXISTS state.markers (
    place     TEXT PRIMARY KEY,
    at_ms     INTEGER NOT NULL DEFAULT 0,
    seen_ms   INTEGER NOT NULL DEFAULT 0,
    device_id TEXT NOT NULL DEFAULT ''
);

-- Resolution looks a key up by kind, which is the only query the journal answers outside export,
-- and export goes by the primary key. Nothing has ever wanted `at_ms` on its own.
CREATE INDEX IF NOT EXISTS state.idx_journal_kind_key ON journal (kind, key);
CREATE INDEX IF NOT EXISTS state.idx_piles_pile ON piles (pile, position);
CREATE INDEX IF NOT EXISTS state.idx_snoozes_due ON snoozes (return_at);
CREATE INDEX IF NOT EXISTS state.idx_notes_thread ON notes (thread_key, deleted);
CREATE INDEX IF NOT EXISTS state.idx_merges_merged ON merges (merged_key);
CREATE INDEX IF NOT EXISTS state.idx_clips_created ON clips (created_at DESC, deleted);
CREATE INDEX IF NOT EXISTS state.idx_rules_destination ON sender_rules (destination);
