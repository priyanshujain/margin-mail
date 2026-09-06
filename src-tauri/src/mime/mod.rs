// Raw RFC 2822 bytes in, one renderable message out.
//
// This module's shape is the seam between the sync engine, which has bytes and wants a row, and the
// reading pane, which has a row and wants something safe to put in an iframe. Everything expensive
// and everything dangerous happens on this side of it: parsing legacy charsets, deciding what is
// quoted, stripping trackers, rewriting `cid:` images into `data:` URIs. Nothing downstream is
// allowed to touch a byte the sender wrote.
//
// `RENDER_VERSION` is stamped on every cached render. Changing the sanitiser means bumping it,
// which invalidates every cached body at once. That is the only honest way to ship a fix to a
// security filter: a cache that survives the fix is a cache that still holds the hole. Version 2
// is the surface decision: every body already in the mirror gets one the next time its thread is
// opened, with no re-download.

pub mod build;
pub mod invite;
pub mod parse;

use std::collections::HashMap;

use crate::dto::{Invite, Person, Surface, Tracker, Unsubscribe};

pub const RENDER_VERSION: i32 = 2;

#[derive(Debug, Clone, Default)]
pub struct RenderedAttachment {
    /// The MIME part's path, which is how the bytes are found again in the raw message.
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub inline: bool,
    pub content_id: Option<String>,
    /// Present when the part was small enough to keep, which is how an inline image becomes a
    /// `data:` URI without a second fetch.
    pub bytes: Option<Vec<u8>>,
}

/// What the caller is willing to let through, and what it has already fetched on the message's
/// behalf. Remote images are never fetched from here: the caller decides, fetches, and passes the
/// bytes back in, so this function has no network and is a pure function of its inputs.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub allow_remote_images: bool,
    pub link_cleaning: bool,
    /// Keyed by the URL as it appeared in the source.
    pub remote_images: HashMap<String, Vec<u8>>,
    /// The account's own addresses. Used only to pick this account's `ATTENDEE` row out of an
    /// invitation, which is how the card knows what it has already answered. Whether a message was
    /// sent by you and whether it was addressed to you are decided by the mirror from the provider's
    /// own flags, not here, so nothing in `Rendered` carries them.
    pub own_addresses: Vec<String>,
}

/// One parsed, sanitised message. Everything the mirror stores about a body and everything the
/// reading pane renders comes from here.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// The first entry of `References`, which is the head of the thread key rule.
    pub references_first: Option<String>,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub reply_to: Vec<Person>,
    pub date_ms: Option<i64>,
    pub subject: String,
    pub snippet: String,

    /// Sanitised, ready for `srcdoc`, with `cid:` images already inlined.
    pub html: String,
    /// The trailing quoted conversation, split off so it can sit behind a pill.
    pub quoted_html: Option<String>,
    /// The plain text of the message: the decoded `text/plain` part when there is one, else the
    /// text of the visible HTML. Deliberately not split at the quote the way `html` is, because
    /// what wants this is the search index and a reply that only quotes is still findable by what
    /// it quoted.
    pub text: Option<String>,
    pub is_html: bool,
    /// Whether the sender painted a page. Decided over the sanitised markup and stored, because
    /// opening a thread is a local read and is not allowed to grow a DOM walk.
    pub surface: Surface,

    pub attachments: Vec<RenderedAttachment>,
    pub trackers: Vec<Tracker>,
    pub blocked_images: u32,
    /// The URLs of the remote images that were blocked, so the caller can fetch them if asked.
    pub blocked_urls: Vec<String>,

    pub invite: Option<Invite>,
    pub unsubscribe: Option<Unsubscribe>,
    pub list_id: Option<String>,
    /// The routing headers the suggestion function reads, kept as they were.
    pub auto_submitted: Option<String>,
    pub precedence: Option<String>,
}

impl Default for Person {
    fn default() -> Self {
        Person {
            name: None,
            address: String::new(),
        }
    }
}

/// The one entry point. Everything else in this module is in service of it.
pub fn render(raw: &[u8], options: &RenderOptions) -> Result<Rendered, String> {
    parse::render(raw, options)
}
