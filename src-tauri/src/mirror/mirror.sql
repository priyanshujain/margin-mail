-- The mirror: a copy of one account's mailbox, derived from the provider, thrown away and rebuilt
-- without losing anything a person decided. Its keys are the provider's ids.
--
-- It holds a window of the mailbox rather than all of it. Rows outside the window are evicted, so
-- every table here is disposable and none of it is a source of truth about a decision.
--
-- No table declares a FOREIGN KEY. Eviction and account deletion sweep in a fixed order in
-- `mirror::evict`, which is the only place referential integrity actually matters, and a REPLACE
-- into `threads` under `foreign_keys=ON` would cascade away the messages it is refreshing.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One provider thread. `thread_key` is the portable key the state database joins on: the first
-- entry of the earliest known message's `References`, else its `In-Reply-To`, else its own
-- `Message-ID`. Kept as its own column beside the provider's id rather than derived on read,
-- because the two are different facts and a client that conflates them cannot follow a
-- conversation across a provider change.
CREATE TABLE IF NOT EXISTS threads (
    provider_thread_id TEXT PRIMARY KEY,
    thread_key         TEXT NOT NULL,
    latest_ms          INTEGER NOT NULL DEFAULT 0,
    message_count      INTEGER NOT NULL DEFAULT 0,
    unseen             INTEGER NOT NULL DEFAULT 0,
    starred            INTEGER NOT NULL DEFAULT 0,
    has_attachment     INTEGER NOT NULL DEFAULT 0,
    has_draft          INTEGER NOT NULL DEFAULT 0,
    in_inbox           INTEGER NOT NULL DEFAULT 1,
    trashed            INTEGER NOT NULL DEFAULT 0,
    spam               INTEGER NOT NULL DEFAULT 0,
    -- Denormalised from the latest message so a list page is one query over one table.
    subject            TEXT NOT NULL DEFAULT '',
    snippet            TEXT NOT NULL DEFAULT '',
    from_name          TEXT,
    from_address       TEXT NOT NULL DEFAULT '',
    participants       TEXT NOT NULL DEFAULT '[]',
    -- Pulled in by a provider search that reached past the window. The next eviction pass removes
    -- it again unless it gained state in the meantime.
    transient          INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS messages (
    id                 TEXT PRIMARY KEY,
    provider_thread_id TEXT NOT NULL,
    thread_key         TEXT NOT NULL,
    -- The RFC header, which is what the state database and a future IMAP mirror both key on.
    message_id         TEXT,
    in_reply_to        TEXT,
    references_first   TEXT,
    date_ms            INTEGER NOT NULL DEFAULT 0,
    from_name          TEXT,
    from_address       TEXT NOT NULL DEFAULT '',
    to_json            TEXT NOT NULL DEFAULT '[]',
    cc_json            TEXT NOT NULL DEFAULT '[]',
    bcc_json           TEXT NOT NULL DEFAULT '[]',
    reply_to_json      TEXT NOT NULL DEFAULT '[]',
    subject            TEXT NOT NULL DEFAULT '',
    snippet            TEXT NOT NULL DEFAULT '',
    seen               INTEGER NOT NULL DEFAULT 0,
    starred            INTEGER NOT NULL DEFAULT 0,
    draft              INTEGER NOT NULL DEFAULT 0,
    sent               INTEGER NOT NULL DEFAULT 0,
    labels             TEXT NOT NULL DEFAULT '[]',
    -- The headers the suggestion function reads. Kept as columns because routing runs over every
    -- message of a first sync and a JSON extract per row is the difference between a second and a
    -- minute.
    list_id            TEXT,
    list_unsubscribe   TEXT,
    list_unsub_post    TEXT,
    auto_submitted     TEXT,
    precedence         TEXT,
    size               INTEGER NOT NULL DEFAULT 0,
    has_attachment     INTEGER NOT NULL DEFAULT 0,
    -- Metadata has been fetched. A row can exist from `messages.list` before it is hydrated.
    hydrated           INTEGER NOT NULL DEFAULT 0,
    transient          INTEGER NOT NULL DEFAULT 0
);

-- Bodies in their own table with a `fetched_at`, so the window can evict them without touching the
-- headers, and so a body is fetched on open rather than during sync. `html` is the sanitised
-- render, cached beside the raw bytes it came from; `render_version` invalidates every cached
-- render at once when the sanitiser changes, which is the only honest way to ship a fix to it.
CREATE TABLE IF NOT EXISTS bodies (
    message_id      TEXT PRIMARY KEY,
    raw             BLOB,
    html            TEXT,
    quoted_html     TEXT,
    text            TEXT,
    is_html         INTEGER NOT NULL DEFAULT 0,
    trackers        TEXT NOT NULL DEFAULT '[]',
    blocked_images  INTEGER NOT NULL DEFAULT 0,
    render_version  INTEGER NOT NULL DEFAULT 0,
    fetched_at      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS attachments (
    id           TEXT PRIMARY KEY,
    message_id   TEXT NOT NULL,
    part_id      TEXT,
    filename     TEXT NOT NULL DEFAULT '',
    mime_type    TEXT NOT NULL DEFAULT 'application/octet-stream',
    size         INTEGER NOT NULL DEFAULT 0,
    inline       INTEGER NOT NULL DEFAULT 0,
    content_id   TEXT,
    -- Fetched on open, never during sync, and capped by the setting.
    cached_path  TEXT,
    cached_at    INTEGER
);

CREATE TABLE IF NOT EXISTS labels (
    id   TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'user'
);

-- Everything on its way out: a send holding its undo delay, a coalesced batch of flag changes, a
-- draft to upload, a one-click unsubscribe POST. Drained by the sync loop, retried with backoff.
CREATE TABLE IF NOT EXISTS outbox (
    id          TEXT PRIMARY KEY,
    op          TEXT NOT NULL,
    payload     TEXT NOT NULL,
    thread_key  TEXT,
    hold_until  INTEGER NOT NULL DEFAULT 0,
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT
);

-- Local drafts, saved as you type. `provider_draft_id` is set once the draft has been uploaded, so
-- a draft roams the way Gmail drafts always have.
CREATE TABLE IF NOT EXISTS drafts (
    id                TEXT PRIMARY KEY,
    provider_draft_id TEXT,
    thread_key        TEXT,
    in_reply_to       TEXT,
    payload           TEXT NOT NULL,
    updated_at        INTEGER NOT NULL DEFAULT 0
);

-- Everyone this account has written to or heard from, ranked for autocomplete. Derived from the
-- mirror, topped up from the People API, and never sent anywhere to be looked up.
CREATE TABLE IF NOT EXISTS correspondents (
    address    TEXT PRIMARY KEY,
    name       TEXT,
    last_ms    INTEGER NOT NULL DEFAULT 0,
    seen_count INTEGER NOT NULL DEFAULT 0,
    sent_count INTEGER NOT NULL DEFAULT 0,
    -- people-api or mirror, so a first run can tell "someone I know" from "someone who wrote once".
    source     TEXT NOT NULL DEFAULT 'mirror'
);

CREATE INDEX IF NOT EXISTS idx_threads_key ON threads (thread_key);
CREATE INDEX IF NOT EXISTS idx_threads_latest ON threads (latest_ms DESC);
CREATE INDEX IF NOT EXISTS idx_threads_from ON threads (from_address);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages (provider_thread_id, date_ms);
CREATE INDEX IF NOT EXISTS idx_messages_key ON messages (thread_key);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages (date_ms DESC);
CREATE INDEX IF NOT EXISTS idx_messages_from ON messages (from_address);
CREATE INDEX IF NOT EXISTS idx_messages_hydrated ON messages (hydrated, date_ms DESC);
CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments (message_id);
CREATE INDEX IF NOT EXISTS idx_outbox_hold ON outbox (hold_until, created_at);
CREATE INDEX IF NOT EXISTS idx_correspondents_rank ON correspondents (last_ms DESC);

-- Standalone rather than an external-content table: the text it indexes is assembled from three
-- tables and a body that arrives later than its headers, so there is no single content table to
-- point at. Rows are written when a body is hydrated and deleted by eviction.
CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(
    message_id UNINDEXED,
    thread_key UNINDEXED,
    subject,
    sender,
    recipients,
    body,
    filenames,
    tokenize = "unicode61 remove_diacritics 2"
);
