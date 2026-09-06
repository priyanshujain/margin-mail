// Raw bytes to a `Rendered`.
//
// The parse is `mail-parser` over the bytes as they came off the wire, never a provider's
// pre-parsed payload: the legacy charsets are the reason. ISO-8859-1 and ISO-2022-JP still arrive
// daily, and a JSON body handed over by an API has already been through somebody else's idea of how
// to turn them into text.
//
// Two things this module does that the parser does not. It joins RFC 2047 encoded words that a
// sender split through the middle of a character before handing the bytes over, because decoding
// each word on its own turns the split character into replacement characters and a subject full of
// those is what the reader sees. And it turns a plain text body into HTML itself rather than taking
// the parser's conversion, because the pane wants paragraphs it can set on its own measure.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use mail_parser::{Address, Message, MessageParser, MimeHeaders, PartType};

use super::invite;
use super::{Rendered, RenderOptions, RenderedAttachment};
use crate::dto::{Person, Surface, Unsubscribe};
use crate::sanitize::{self, html, quoted, surface, InlinePart, Policy};

/// Attachment bytes small enough to keep beside the row, so opening one costs no second fetch.
const KEEP_BYTES_UNDER: usize = 256 * 1024;

/// How much of the body the list shows under the subject.
const SNIPPET_CHARS: usize = 200;

pub fn render(raw: &[u8], options: &RenderOptions) -> Result<Rendered, String> {
    let repaired = join_split_encoded_words(raw);
    let bytes = repaired.as_deref().unwrap_or(raw);
    let message = MessageParser::default()
        .parse(bytes)
        .ok_or_else(|| "the message could not be parsed at all".to_string())?;

    let policy = Policy {
        allow_remote_images: options.allow_remote_images,
        link_cleaning: options.link_cleaning,
    };

    let invite = calendar_invite(&message, &options.own_addresses)?;
    let inline_parts = inline_parts(&message);

    let (source, is_html, text) = body(&message);
    let (visible_source, quoted_source) = if is_html {
        quoted::split_html(&source)
    } else {
        let (visible, quoted) = quoted::split_text(&source);
        (html::from_text(&visible), quoted.map(|part| html::from_text(&part)))
    };

    let mut sanitized = sanitize::sanitize(&visible_source, &inline_parts, &options.remote_images, policy)?;
    let mut quoted_html = match &quoted_source {
        Some(source) => {
            let quoted = sanitize::sanitize(source, &inline_parts, &options.remote_images, policy)?;
            sanitized.absorb_counts(&quoted);
            Some(quoted.html)
        }
        None => None,
    };

    // The visible part decides, because that is the part with the shell on it, and both parts then
    // get the same answer: one message, one surface, whether or not the quote is open. A body this
    // app wrote itself is never on a sender's page, so plain text skips the question entirely.
    let surface = if is_html {
        surface::decide(&sanitized.html, &visible_source)
    } else {
        Surface::Theme
    };
    if surface == Surface::Theme {
        sanitized.html = surface::neutralise(&sanitized.html);
        quoted_html = quoted_html.map(|html| surface::neutralise(&html));
    }

    let used: HashSet<&str> = sanitized
        .used_content_ids
        .iter()
        .map(String::as_str)
        .collect();

    Ok(Rendered {
        message_id: message.message_id().map(str::to_string),
        in_reply_to: first_id(message.in_reply_to()),
        references_first: first_id(message.references()),
        from: people(message.from()).into_iter().next().unwrap_or_default(),
        to: people(message.to()),
        cc: people(message.cc()),
        bcc: people(message.bcc()),
        reply_to: people(message.reply_to()),
        date_ms: message.date().map(|date| date.to_timestamp() * 1000),
        subject: message.subject().unwrap_or_default().to_string(),
        snippet: snippet(&sanitized.html),
        html: sanitized.html.clone(),
        quoted_html,
        text: text.or_else(|| Some(sanitize::text_of(&sanitized.html))),
        is_html,
        surface,
        attachments: attachments(&message, &used, invite.is_some()),
        trackers: sanitized.trackers,
        blocked_images: sanitized.blocked_images,
        blocked_urls: sanitized.blocked_urls,
        invite,
        unsubscribe: unsubscribe(&message),
        list_id: bracketed(message.header_raw("List-Id")),
        auto_submitted: raw_header(&message, "Auto-Submitted"),
        precedence: raw_header(&message, "Precedence"),
    })
}

// -------------------------------------------------------------------------------------------
// Encoded words a sender cut in half
// -------------------------------------------------------------------------------------------

/// Adjacent encoded words that share a charset and an encoding, joined into one before the parser
/// sees them.
///
/// RFC 2047 section 5 says a multi-byte character may not be split across two encoded words, and
/// senders do it anyway: half of `ê` in one base64 word and half in the next. Decoding each word on
/// its own gives two replacement characters where a letter should be, so the join happens first and
/// the decoding happens once. Only the header blocks are touched, found from the parse rather than
/// guessed at, because `=?...?=` in a message body is text somebody wrote about encoded words.
///
/// Returns `None` when nothing needed joining, which is almost every message.
fn join_split_encoded_words(raw: &[u8]) -> Option<Vec<u8>> {
    let message = MessageParser::default().parse(raw)?;
    let mut ranges: Vec<(usize, usize)> = message
        .parts
        .iter()
        .map(|part| (part.offset_header as usize, part.offset_body as usize))
        .filter(|(start, end)| start < end && *end <= raw.len())
        .collect();
    ranges.sort_unstable();
    drop(message);

    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut cursor = 0usize;
    let mut changed = false;
    for (start, end) in ranges {
        if start < cursor {
            continue;
        }
        out.extend_from_slice(&raw[cursor..start]);
        let joined = join_in_block(&raw[start..end]);
        changed |= joined.len() != end - start;
        out.extend_from_slice(&joined);
        cursor = end;
    }
    out.extend_from_slice(&raw[cursor..]);
    changed.then_some(out)
}

struct EncodedWord {
    charset: String,
    encoding: u8,
    text: Vec<u8>,
    start: usize,
    end: usize,
}

fn join_in_block(block: &[u8]) -> Vec<u8> {
    let words = encoded_words(block);
    let mut out = Vec::with_capacity(block.len());
    let mut cursor = 0usize;
    let mut index = 0usize;

    while index < words.len() {
        let mut last = index;
        while last + 1 < words.len()
            && words[last + 1].charset.eq_ignore_ascii_case(&words[index].charset)
            && words[last + 1].encoding == words[index].encoding
            && only_whitespace(&block[words[last].end..words[last + 1].start])
        {
            last += 1;
        }
        if last == index {
            index += 1;
            continue;
        }

        let mut decoded: Vec<u8> = Vec::new();
        for word in &words[index..=last] {
            match decode_word(word) {
                Some(bytes) => decoded.extend_from_slice(&bytes),
                None => {
                    decoded.clear();
                    break;
                }
            }
        }
        if decoded.is_empty() {
            index = last + 1;
            continue;
        }

        out.extend_from_slice(&block[cursor..words[index].start]);
        out.extend_from_slice(
            format!(
                "=?{}?B?{}?=",
                words[index].charset,
                base64::engine::general_purpose::STANDARD.encode(&decoded)
            )
            .as_bytes(),
        );
        cursor = words[last].end;
        index = last + 1;
    }
    out.extend_from_slice(&block[cursor..]);
    out
}

fn encoded_words(block: &[u8]) -> Vec<EncodedWord> {
    let mut words = Vec::new();
    let mut i = 0usize;
    while i + 4 < block.len() {
        if &block[i..i + 2] != b"=?" {
            i += 1;
            continue;
        }
        let Some(charset_end) = block[i + 2..].iter().position(|byte| *byte == b'?') else {
            break;
        };
        let charset_end = i + 2 + charset_end;
        let Some(encoding) = block.get(charset_end + 1).copied() else {
            break;
        };
        if block.get(charset_end + 2) != Some(&b'?') || !matches!(encoding | 0x20, b'b' | b'q') {
            i += 2;
            continue;
        }
        let text_start = charset_end + 3;
        let Some(text_end) = find_pair(block, text_start, b"?=") else {
            break;
        };
        words.push(EncodedWord {
            charset: String::from_utf8_lossy(&block[i + 2..charset_end]).to_string(),
            encoding: encoding.to_ascii_uppercase(),
            text: block[text_start..text_end].to_vec(),
            start: i,
            end: text_end + 2,
        });
        i = text_end + 2;
    }
    words
}

fn decode_word(word: &EncodedWord) -> Option<Vec<u8>> {
    match word.encoding {
        b'B' => {
            let text: Vec<u8> = word
                .text
                .iter()
                .copied()
                .filter(|byte| !byte.is_ascii_whitespace())
                .collect();
            base64::engine::general_purpose::STANDARD
                .decode(&text)
                .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&text))
                .ok()
        }
        _ => {
            let mut out = Vec::with_capacity(word.text.len());
            let mut i = 0usize;
            while i < word.text.len() {
                match word.text[i] {
                    b'_' => out.push(b' '),
                    b'=' if i + 2 < word.text.len() => {
                        let hex = std::str::from_utf8(&word.text[i + 1..i + 3]).ok()?;
                        out.push(u8::from_str_radix(hex, 16).ok()?);
                        i += 2;
                    }
                    byte => out.push(byte),
                }
                i += 1;
            }
            Some(out)
        }
    }
}

fn find_pair(haystack: &[u8], from: usize, needle: &[u8; 2]) -> Option<usize> {
    (from..haystack.len().saturating_sub(1)).find(|at| &haystack[*at..*at + 2] == needle)
}

/// Whitespace and folding only, which is what RFC 2047 allows between two adjacent encoded words.
fn only_whitespace(between: &[u8]) -> bool {
    !between.is_empty() && between.iter().all(|byte| byte.is_ascii_whitespace())
        || between.is_empty()
}

// -------------------------------------------------------------------------------------------
// Headers
// -------------------------------------------------------------------------------------------

fn people(address: Option<&Address>) -> Vec<Person> {
    let mut people = Vec::new();
    match address {
        Some(Address::List(list)) => {
            for entry in list {
                push_person(&mut people, entry.name.as_deref(), entry.address.as_deref());
            }
        }
        Some(Address::Group(groups)) => {
            for group in groups {
                for entry in &group.addresses {
                    push_person(&mut people, entry.name.as_deref(), entry.address.as_deref());
                }
            }
        }
        None => {}
    }
    people
}

fn push_person(people: &mut Vec<Person>, name: Option<&str>, address: Option<&str>) {
    let Some(address) = address.map(str::trim).filter(|address| address.contains('@')) else {
        return;
    };
    people.push(Person {
        name: name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
        address: address.to_string(),
    });
}

fn first_id(value: &mail_parser::HeaderValue) -> Option<String> {
    match value {
        mail_parser::HeaderValue::Text(text) => Some(text.trim().to_string()),
        mail_parser::HeaderValue::TextList(list) => {
            list.first().map(|text| text.trim().to_string())
        }
        _ => None,
    }
}

fn raw_header(message: &Message, name: &str) -> Option<String> {
    message
        .header_raw(name)
        .map(|value| unfold(value))
        .filter(|value| !value.is_empty())
}

fn unfold(value: &str) -> String {
    value
        .split(|character| character == '\r' || character == '\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The identifier inside the angle brackets, which is what a `List-Id` is once the human readable
/// part in front of it is taken off.
fn bracketed(value: Option<&str>) -> Option<String> {
    let value = unfold(value?);
    match (value.rfind('<'), value.rfind('>')) {
        (Some(open), Some(close)) if close > open + 1 => Some(value[open + 1..close].to_string()),
        _ => Some(value).filter(|value| !value.is_empty()),
    }
}

fn unsubscribe(message: &Message) -> Option<Unsubscribe> {
    let raw = raw_header(message, "List-Unsubscribe")?;
    let mut mailto = None;
    let mut url = None;
    for entry in raw.split(',') {
        let entry = entry.trim().trim_start_matches('<').trim_end_matches('>').trim();
        if entry.starts_with("mailto:") && mailto.is_none() {
            mailto = Some(entry.to_string());
        } else if (entry.starts_with("http://") || entry.starts_with("https://")) && url.is_none() {
            url = Some(entry.to_string());
        }
    }
    if mailto.is_none() && url.is_none() {
        return None;
    }
    Some(Unsubscribe {
        one_click: raw_header(message, "List-Unsubscribe-Post")
            .map(|value| value.to_ascii_lowercase().contains("one-click"))
            .unwrap_or(false)
            && url.is_some(),
        mailto,
        url,
    })
}

// -------------------------------------------------------------------------------------------
// Parts
// -------------------------------------------------------------------------------------------

/// The body to render: the HTML alternative when there is one, the plain text otherwise, and the
/// decoded plain text alongside it either way when the message carried one.
fn body(message: &Message) -> (String, bool, Option<String>) {
    let text = message
        .text_body
        .iter()
        .filter_map(|id| message.parts.get(*id as usize))
        .filter(|part| !is_calendar(part))
        .find_map(|part| match &part.body {
            PartType::Text(text) => Some(text.to_string()),
            _ => None,
        });

    let html = message
        .html_body
        .iter()
        .filter_map(|id| message.parts.get(*id as usize))
        .find_map(|part| match &part.body {
            PartType::Html(html) => Some(html.to_string()),
            _ => None,
        });

    match html {
        Some(html) => (html, true, text),
        None => (text.clone().unwrap_or_default(), false, text),
    }
}

fn is_calendar(part: &mail_parser::MessagePart) -> bool {
    part.content_type()
        .map(|content_type| {
            let subtype = content_type.subtype().unwrap_or_default();
            subtype.eq_ignore_ascii_case("calendar") || subtype.eq_ignore_ascii_case("ics")
        })
        .unwrap_or(false)
}

/// The parts a `cid:` reference can point at.
///
/// There is no lookup by Content-ID in `mail-parser`, so the parts are walked and each one's
/// `Content-ID` is read off. That is the whole of it: the header is per part and the map is built
/// once.
fn inline_parts(message: &Message) -> HashMap<String, InlinePart> {
    let mut parts = HashMap::new();
    for part in &message.parts {
        let Some(content_id) = part.content_id() else {
            continue;
        };
        let bytes = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.to_vec(),
            _ => continue,
        };
        let mime_type = match part.content_type() {
            Some(content_type) => format!(
                "{}/{}",
                content_type.ctype(),
                content_type.subtype().unwrap_or_default()
            ),
            None => "application/octet-stream".to_string(),
        };
        parts.insert(
            content_id.trim_matches(|c| c == '<' || c == '>').to_string(),
            InlinePart {
                mime_type: mime_type.to_ascii_lowercase(),
                bytes,
            },
        );
    }
    parts
}

fn attachments(
    message: &Message,
    used_content_ids: &HashSet<&str>,
    has_invite: bool,
) -> Vec<RenderedAttachment> {
    let paths = part_paths(message);
    let bodies: HashSet<usize> = message
        .html_body
        .iter()
        .chain(message.text_body.iter())
        .map(|id| *id as usize)
        .collect();

    let mut attachments = Vec::new();
    for (index, part) in message.parts.iter().enumerate() {
        if bodies.contains(&index) {
            continue;
        }
        let bytes = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.to_vec(),
            PartType::Text(text) | PartType::Html(text) => text.as_bytes().to_vec(),
            PartType::Multipart(_) | PartType::Message(_) => continue,
        };
        let content_id = part
            .content_id()
            .map(|id| id.trim_matches(|c| c == '<' || c == '>').to_string());
        if content_id
            .as_deref()
            .map(|id| used_content_ids.contains(id))
            .unwrap_or(false)
        {
            continue;
        }
        // The invite card is the affordance for a calendar part, and a chip beside it is the same
        // event a second time with a worse way to answer it.
        if has_invite && is_calendar(part) {
            continue;
        }

        let mime_type = match part.content_type() {
            Some(content_type) => format!(
                "{}/{}",
                content_type.ctype().to_ascii_lowercase(),
                content_type
                    .subtype()
                    .unwrap_or("octet-stream")
                    .to_ascii_lowercase()
            ),
            None => "application/octet-stream".to_string(),
        };
        let part_id = paths.get(&index).cloned().unwrap_or_else(|| index.to_string());
        attachments.push(RenderedAttachment {
            filename: part
                .attachment_name()
                .map(str::to_string)
                .unwrap_or_else(|| default_filename(&part_id, &mime_type)),
            mime_type,
            size: bytes.len() as u64,
            inline: part
                .content_disposition()
                .map(|disposition| disposition.ctype().eq_ignore_ascii_case("inline"))
                .unwrap_or(false)
                || content_id.is_some(),
            content_id,
            bytes: (bytes.len() <= KEEP_BYTES_UNDER).then_some(bytes),
            part_id,
        });
    }
    attachments
}

fn default_filename(part_id: &str, mime_type: &str) -> String {
    let extension = mime_type.rsplit('/').next().unwrap_or("bin");
    format!("part-{part_id}.{extension}")
}

/// The IMAP style path to each part, which is how its bytes are found again in the raw message.
fn part_paths(message: &Message) -> HashMap<usize, String> {
    let mut paths = HashMap::new();
    match message.parts.first().map(|part| &part.body) {
        Some(PartType::Multipart(children)) => {
            paths.insert(0, String::new());
            walk_paths(message, children, "", &mut paths);
        }
        Some(_) => {
            paths.insert(0, "1".to_string());
        }
        None => {}
    }
    paths
}

fn walk_paths(
    message: &Message,
    children: &[u32],
    prefix: &str,
    paths: &mut HashMap<usize, String>,
) {
    for (position, id) in children.iter().enumerate() {
        let index = *id as usize;
        let path = if prefix.is_empty() {
            format!("{}", position + 1)
        } else {
            format!("{prefix}.{}", position + 1)
        };
        if let Some(PartType::Multipart(grandchildren)) =
            message.parts.get(index).map(|part| &part.body)
        {
            walk_paths(message, grandchildren, &path, paths);
        }
        paths.insert(index, path);
    }
}

fn calendar_invite(
    message: &Message,
    own_addresses: &[String],
) -> Result<Option<crate::dto::Invite>, String> {
    for part in &message.parts {
        if !is_calendar(part) {
            continue;
        }
        let text = match &part.body {
            PartType::Text(text) => text.to_string(),
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => {
                String::from_utf8_lossy(bytes).to_string()
            }
            _ => continue,
        };
        if let Some(invite) = invite::parse(&text, own_addresses)? {
            return Ok(Some(invite));
        }
    }
    Ok(None)
}

fn snippet(html: &str) -> String {
    let text = sanitize::text_of(html);
    if text.chars().count() <= SNIPPET_CHARS {
        return text;
    }
    let cut = text
        .char_indices()
        .nth(SNIPPET_CHARS)
        .map(|(at, _)| at)
        .unwrap_or(text.len());
    let trimmed = &text[..cut];
    let end = trimmed.rfind(' ').unwrap_or(cut);
    format!(
        "{}…",
        trimmed[..end].trim_end_matches([' ', '.', ',', ';', ':'])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::InviteResponse;
    use crate::mime::RENDER_VERSION;
    use sha2::{Digest, Sha256};

    /// The corpus, compiled in. It is loaded here rather than through `crate::fixtures` because the
    /// corpus belongs to the package that wrote it and this package only reads it.
    macro_rules! corpus {
        ($($file:literal,)+) => {
            fn corpus() -> Vec<(&'static str, &'static [u8])> {
                vec![$(($file, include_bytes!(concat!("../../fixtures/", $file)) as &[u8]),)+]
            }
        };
    }

    corpus! {
        "attachment-filename-encoded-word.eml",
        "calendar-invite.eml",
        "charset-iso-2022-jp.eml",
        "charset-iso-8859-1.eml",
        "cid-missing-part.eml",
        "display-name-comma-quotes.eml",
        "encoded-words-split-utf8.eml",
        "encoded-words-subject-b.eml",
        "encoded-words-subject-q.eml",
        "group-address-list.eml",
        "header-oddities.eml",
        "html-hostile.eml",
        "inline-image-attachment-disposition.eml",
        "inline-image-cid.eml",
        "invite-daylight-saving.eml",
        "nested-multipart.eml",
        "newsletter-list-unsubscribe.eml",
        "no-message-id.eml",
        "outlook-reply.eml",
        "plain-text.eml",
        "quoted-printable-soft-breaks.eml",
        "quoted-reply-blockquote.eml",
        "quoted-reply-plain.eml",
        "receipt-no-reply.eml",
        "reply-references-folded.eml",
        "rfc2231-filename.eml",
        "screener-first-contact.eml",
        "service-on-behalf.eml",
        "surface-dark-inline-text.eml",
        "surface-newsletter-background-image.eml",
        "surface-newsletter-bgcolor.eml",
        "surface-plain-html.eml",
        "surface-sender-dark-design.eml",
        "surface-wrapper-background.eml",
        "tracking-pixel.eml",
        "utf8-raw-headers.eml",
    }

    fn find(name: &str) -> &'static [u8] {
        corpus()
            .into_iter()
            .find(|(file, _)| *file == name)
            .map(|(_, raw)| raw)
            .unwrap_or_else(|| panic!("{name} is not in the corpus"))
    }

    /// The options every golden is written under. Images off and cleaning on is what the app opens
    /// a message with, so it is the state worth pinning.
    fn reading_options() -> RenderOptions {
        RenderOptions {
            allow_remote_images: false,
            link_cleaning: true,
            remote_images: HashMap::new(),
            own_addresses: vec!["pj@73ai.org".to_string()],
        }
    }

    fn rendered(name: &str) -> Rendered {
        render(find(name), &reading_options()).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    // -------------------------------------------------------------------------------------
    // The golden corpus
    // -------------------------------------------------------------------------------------

    /// Regenerating a golden is a review, not a fix. `MARGIN_MAIL_BLESS_GOLDEN=1 cargo test`
    /// rewrites every file under `fixtures/golden/`, and the diff it produces is the thing to read:
    /// a golden nobody read is a test that asserts today's bug back at you. The sanitiser is the
    /// only security boundary in this app, so a change to one of these files is a change to what a
    /// message is allowed to do, and it wants the same attention as the code that caused it.
    #[test]
    fn every_fixture_matches_its_golden() {
        let bless = std::env::var("MARGIN_MAIL_BLESS_GOLDEN").is_ok();
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/golden");
        let mut wrong = Vec::new();

        for (name, raw) in corpus() {
            let golden = directory.join(name.replace(".eml", ".txt"));
            let actual = describe(&render(raw, &reading_options()).expect(name));
            if bless {
                std::fs::create_dir_all(&directory).expect("the golden directory");
                std::fs::write(&golden, &actual).expect("a golden");
                continue;
            }
            match std::fs::read_to_string(&golden) {
                Ok(expected) if expected == actual => {}
                Ok(expected) => wrong.push(format!("{name}\n{}", first_difference(&expected, &actual))),
                Err(error) => wrong.push(format!("{name}: {error}")),
            }
        }

        assert!(
            wrong.is_empty(),
            "{} golden file(s) do not match. Read the change before blessing it with \
             MARGIN_MAIL_BLESS_GOLDEN=1.\n\n{}",
            wrong.len(),
            wrong.join("\n\n")
        );
    }

    /// The surface every fixture lands on, asserted by name rather than left to the goldens.
    ///
    /// A golden says what changed; this says what the rule is. The two are worth having separately,
    /// because a golden that quietly flips from `paper` to `theme` is a line in a diff and this is a
    /// failing test with a name on it.
    #[test]
    fn every_fixture_lands_on_the_surface_the_rule_says_it_should() {
        // Everything not named here paints nothing, which is almost every message anybody is sent:
        // the whole corpus above this line is replies, receipts, invitations and a newsletter
        // written as prose, and not one of them wants a white slab in a dark window.
        let paper = [
            // A table shell with a wash behind a 600 pixel card, which is a designed page.
            "surface-newsletter-bgcolor.eml",
            // The same thing painted with an image instead. The sanitiser takes the image out
            // because a URL is a fetch, so this one is decided from the source.
            "surface-newsletter-background-image.eml",
            // One wrapper `<div>` with a `background-color` on it, which is the smallest thing
            // that still counts as painting a page.
            "surface-wrapper-background.eml",
        ];

        for (name, raw) in corpus() {
            let rendered = render(raw, &reading_options()).expect(name);
            let expected = if paper.contains(&name) {
                Surface::Paper
            } else {
                Surface::Theme
            };
            assert_eq!(rendered.surface, expected, "{name}");
        }
    }

    #[test]
    fn a_body_on_the_theme_keeps_the_colours_that_read_and_loses_the_ones_that_do_not() {
        let rendered = rendered("surface-dark-inline-text.eml");
        assert_eq!(rendered.surface, Surface::Theme);
        // Near black on a dark page is the whole complaint, so it goes and the text inherits ours.
        assert!(!rendered.html.contains("#222222"));
        assert!(!rendered.html.contains("#1c1c1c"));
        assert!(!rendered.html.contains("color=\"#000000\""));
        // A mid tone reads on both of our papers, so it is the sender's to keep.
        assert!(rendered.html.contains("#d32f2f"));
        assert!(rendered.html.contains("#666666"));
    }

    /// A sender who wrote a dark mode into their own stylesheet still gets the neutraliser.
    ///
    /// This is the one place the brief and the sanitiser disagree, and the sanitiser wins. The idea
    /// was to read a `prefers-color-scheme` block as the sender saying "I have handled dark, leave
    /// me alone", which is right for Thunderbird because Thunderbird keeps the stylesheet. Here
    /// `ammonia` drops `<style>` with its content, so by the time anything could honour that
    /// promise the rules that would have kept it are gone and what is left is the light mode
    /// colours written inline for Outlook. Skipping the neutraliser on this message would leave
    /// `#1f1f1f` body text on our dark page, which is exactly the bug being fixed.
    #[test]
    fn a_sender_who_shipped_their_own_dark_mode_still_has_it_taken_out_of_their_hands() {
        let rendered = rendered("surface-sender-dark-design.eml");
        assert_eq!(rendered.surface, Surface::Theme);
        assert!(!rendered.html.contains("prefers-color-scheme"));
        assert!(!rendered.html.contains("#1f1f1f"));
        assert!(rendered.html.contains("#d32f2f"));
    }

    #[test]
    fn a_painted_page_is_left_exactly_as_the_sender_drew_it() {
        let rendered = rendered("surface-newsletter-bgcolor.eml");
        assert_eq!(rendered.surface, Surface::Paper);
        // Nothing is neutralised on this branch: the page it was drawn for is the page it gets.
        assert!(rendered.html.contains("bgcolor=\"#ffffff\""));
        assert!(rendered.html.contains("#333333"));
        assert!(rendered.html.contains("#1a1a1a"));
    }

    fn first_difference(expected: &str, actual: &str) -> String {
        for (line, (want, got)) in expected.lines().zip(actual.lines()).enumerate() {
            if want != got {
                return format!("  line {}\n  want: {want}\n  got:  {got}", line + 1);
            }
        }
        format!(
            "  the files agree for {} lines and then one of them stops (want {} lines, got {})",
            expected.lines().count().min(actual.lines().count()),
            expected.lines().count(),
            actual.lines().count()
        )
    }

    /// A golden is a person's job to read, so it is written for a person: one field per line, the
    /// bodies last, and attachment bytes as a digest because nobody reviews base64.
    fn describe(rendered: &Rendered) -> String {
        let mut out = String::new();
        let line = |out: &mut String, name: &str, value: &str| {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(if value.is_empty() { "-" } else { value });
            out.push('\n');
        };
        let person = |person: &Person| match &person.name {
            Some(name) => format!("{name} <{}>", person.address),
            None => format!("<{}>", person.address),
        };
        let people = |people: &[Person]| {
            people
                .iter()
                .map(person)
                .collect::<Vec<_>>()
                .join(", ")
        };

        line(&mut out, "render-version", &RENDER_VERSION.to_string());
        line(&mut out, "message-id", rendered.message_id.as_deref().unwrap_or_default());
        line(&mut out, "in-reply-to", rendered.in_reply_to.as_deref().unwrap_or_default());
        line(&mut out, "references-first", rendered.references_first.as_deref().unwrap_or_default());
        line(&mut out, "from", &person(&rendered.from));
        line(&mut out, "to", &people(&rendered.to));
        line(&mut out, "cc", &people(&rendered.cc));
        line(&mut out, "bcc", &people(&rendered.bcc));
        line(&mut out, "reply-to", &people(&rendered.reply_to));
        line(&mut out, "date-ms", &rendered.date_ms.map(|ms| ms.to_string()).unwrap_or_default());
        line(&mut out, "subject", &rendered.subject);
        line(&mut out, "is-html", &rendered.is_html.to_string());
        line(
            &mut out,
            "surface",
            match rendered.surface {
                Surface::Theme => "theme",
                Surface::Paper => "paper",
            },
        );
        line(&mut out, "snippet", &rendered.snippet);
        line(&mut out, "list-id", rendered.list_id.as_deref().unwrap_or_default());
        line(&mut out, "auto-submitted", rendered.auto_submitted.as_deref().unwrap_or_default());
        line(&mut out, "precedence", rendered.precedence.as_deref().unwrap_or_default());
        line(
            &mut out,
            "unsubscribe",
            &rendered
                .unsubscribe
                .as_ref()
                .map(|unsubscribe| {
                    format!(
                        "one-click={} mailto={} url={}",
                        unsubscribe.one_click,
                        unsubscribe.mailto.as_deref().unwrap_or("-"),
                        unsubscribe.url.as_deref().unwrap_or("-")
                    )
                })
                .unwrap_or_default(),
        );
        line(&mut out, "blocked-images", &rendered.blocked_images.to_string());
        for url in &rendered.blocked_urls {
            line(&mut out, "blocked-url", url);
        }
        for tracker in &rendered.trackers {
            line(&mut out, "tracker", &format!("{} {}", tracker.vendor, tracker.url));
        }
        for attachment in &rendered.attachments {
            line(
                &mut out,
                "attachment",
                &format!(
                    "part={} name={:?} type={} size={} inline={} cid={} sha256={}",
                    attachment.part_id,
                    attachment.filename,
                    attachment.mime_type,
                    attachment.size,
                    attachment.inline,
                    attachment.content_id.as_deref().unwrap_or("-"),
                    attachment
                        .bytes
                        .as_ref()
                        .map(|bytes| digest(bytes))
                        .unwrap_or_else(|| "not-kept".to_string())
                ),
            );
        }
        if let Some(invite) = &rendered.invite {
            line(&mut out, "invite-uid", &invite.uid);
            line(&mut out, "invite-summary", &invite.summary);
            line(&mut out, "invite-start-ms", &invite.start_ms.to_string());
            line(&mut out, "invite-end-ms", &invite.end_ms.to_string());
            line(&mut out, "invite-all-day", &invite.all_day.to_string());
            line(&mut out, "invite-location", invite.location.as_deref().unwrap_or_default());
            line(
                &mut out,
                "invite-organizer",
                &invite.organizer.as_ref().map(person).unwrap_or_default(),
            );
            line(&mut out, "invite-my-response", &format!("{:?}", invite.my_response));
            line(&mut out, "invite-description", &invite.description.clone().unwrap_or_default().replace('\n', "\\n"));
        }

        out.push_str("\n--- html ---\n");
        out.push_str(rendered.html.trim_end());
        out.push('\n');
        match &rendered.quoted_html {
            Some(quoted) => {
                out.push_str("\n--- quoted ---\n");
                out.push_str(quoted.trim_end());
                out.push('\n');
            }
            None => out.push_str("\n--- quoted: none ---\n"),
        }
        out.push_str("\n--- text ---\n");
        out.push_str(rendered.text.as_deref().unwrap_or("-").trim_end());
        out.push('\n');
        out
    }

    fn digest(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())[..16].to_string()
    }

    // -------------------------------------------------------------------------------------
    // What a golden diff would let slide
    // -------------------------------------------------------------------------------------

    fn every_option_set() -> Vec<RenderOptions> {
        let mut with_images = reading_options();
        with_images.allow_remote_images = true;
        let mut no_cleaning = reading_options();
        no_cleaning.link_cleaning = false;
        let mut everything = reading_options();
        everything.allow_remote_images = true;
        everything.link_cleaning = false;
        vec![
            RenderOptions::default(),
            reading_options(),
            with_images,
            no_cleaning,
            everything,
        ]
    }

    #[test]
    fn nothing_in_the_corpus_can_fetch_run_or_navigate_to_code() {
        for (name, raw) in corpus() {
            for options in every_option_set() {
                // Every remote URL is also handed back as a fetched image, so the branch that
                // inlines one is exercised rather than skipped for want of bytes.
                let mut options = options;
                let first = render(raw, &options).unwrap_or_else(|error| panic!("{name}: {error}"));
                for url in &first.blocked_urls {
                    options
                        .remote_images
                        .insert(url.clone(), b"\x89PNG\r\n\x1a\nrest".to_vec());
                }
                let rendered = render(raw, &options).unwrap_or_else(|error| panic!("{name}: {error}"));

                for body in [Some(&rendered.html), rendered.quoted_html.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    let lowercase = body.to_ascii_lowercase();
                    for forbidden in [
                        "<script", "<iframe", "<form", "<object", "<embed", "<meta", "<link",
                        "<base", "javascript:", "vbscript:", "data:text/html",
                    ] {
                        assert!(
                            !lowercase.contains(forbidden),
                            "{name} produced {forbidden:?} in\n{body}"
                        );
                    }
                    for tag in crate::sanitize::scan(body) {
                        if let Some(style) = tag.attr("style") {
                            assert!(
                                !style.to_ascii_lowercase().contains("url("),
                                "{name} kept a url() in a style attribute: {style}"
                            );
                        }
                        for (attribute, value) in &tag.attrs {
                            assert!(
                                !attribute.starts_with("on"),
                                "{name} kept the handler {attribute}"
                            );
                            if tag.name == "img" && attribute == "src" {
                                assert!(
                                    value.starts_with("data:image/"),
                                    "{name} left {value} in an img src"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn render_is_a_pure_function_of_its_inputs() {
        for (name, raw) in corpus() {
            let options = reading_options();
            let first = describe(&render(raw, &options).expect(name));
            let second = describe(&render(raw, &options).expect(name));
            assert_eq!(first, second, "{name} rendered differently twice");
        }
    }

    #[test]
    fn a_tracking_pixel_is_named_and_the_photograph_beside_it_is_only_blocked() {
        let rendered = rendered("tracking-pixel.eml");
        let vendors: Vec<&str> = rendered
            .trackers
            .iter()
            .map(|tracker| tracker.vendor.as_str())
            .collect();
        assert_eq!(vendors, vec!["HubSpot", "meridianproperties.in"]);
        assert_eq!(rendered.blocked_images, 1);
        assert_eq!(
            rendered.blocked_urls,
            vec!["https://meridianproperties.in/img/studio-floorplan.jpg".to_string()]
        );
    }

    #[test]
    fn an_encoded_word_subject_comes_out_as_the_letters_it_stands_for() {
        assert_eq!(
            rendered("encoded-words-subject-b.eml").subject,
            "Les Misérables tickets, second night"
        );
        assert_eq!(
            rendered("encoded-words-subject-q.eml").subject,
            "Angebot: Küchenarbeitsplatte in Eiche massiv (Nachtrag)"
        );
        assert_eq!(
            rendered("encoded-words-subject-q.eml").from.name.as_deref(),
            Some("Lena Brandt (Tischlerei München)")
        );
    }

    /// The sender split `ê` across two base64 encoded words, which RFC 2047 forbids and senders do
    /// anyway. Decoded word by word it is two replacement characters.
    #[test]
    fn a_character_split_across_two_encoded_words_is_still_one_character() {
        let rendered = rendered("encoded-words-split-utf8.eml");
        assert_eq!(
            rendered.subject,
            "Your reservation in Lisbon is confirmed, Inês is expecting you"
        );
        assert!(!rendered.subject.contains('\u{fffd}'), "{}", rendered.subject);
        assert_eq!(rendered.reply_to[0].name.as_deref(), Some("Inês"));
    }

    #[test]
    fn the_legacy_charsets_come_out_as_unicode() {
        let french = rendered("charset-iso-8859-1.eml");
        assert_eq!(french.subject, "Votre réservation est confirmée");
        assert_eq!(french.from.name.as_deref(), Some("Théâtre du Châtelet"));
        assert!(french.html.contains("côté jardin"), "{}", french.html);

        let japanese = rendered("charset-iso-2022-jp.eml");
        assert_eq!(japanese.subject, "会議の資料を送ります");
        assert_eq!(japanese.from.name.as_deref(), Some("佐藤 花子"));
        assert!(japanese.html.contains("佐藤"), "{}", japanese.html);
    }

    #[test]
    fn an_rfc_2231_filename_arrives_whole() {
        let rendered = rendered("rfc2231-filename.eml");
        assert_eq!(rendered.attachments.len(), 1);
        assert_eq!(
            rendered.attachments[0].filename,
            "Angebot Küchenarbeitsplatte Eiche massiv 2026.pdf"
        );
    }

    #[test]
    fn an_encoded_word_filename_arrives_whole_too() {
        let rendered = rendered("attachment-filename-encoded-word.eml");
        assert_eq!(rendered.attachments.len(), 1);
        assert_eq!(
            rendered.attachments[0].filename,
            "Studio lease 2026 (signé).pdf"
        );
    }

    #[test]
    fn an_inline_image_is_shown_once_and_is_not_also_a_chip() {
        let rendered = rendered("inline-image-cid.eml");
        assert_eq!(rendered.html.matches("data:image/png;base64,").count(), 2);
        assert!(!rendered.html.contains("cid:"), "{}", rendered.html);
        assert!(
            rendered.attachments.is_empty(),
            "{:?}",
            rendered.attachments
        );
        assert_eq!(rendered.blocked_images, 0);
    }

    #[test]
    fn a_reply_is_split_where_the_quote_starts() {
        let html = rendered("quoted-reply-blockquote.eml");
        assert!(html.html.contains("plus the reading list"), "{}", html.html);
        assert!(!html.html.contains("Good talk"), "{}", html.html);
        let quoted = html.quoted_html.expect("a quote");
        assert!(quoted.contains("Good talk"), "{quoted}");
        assert!(quoted.contains("Talk is on Monday"), "{quoted}");

        let plain = rendered("quoted-reply-plain.eml");
        assert!(plain.html.contains("Wednesdays at five"), "{}", plain.html);
        assert!(!plain.html.contains("form signed"), "{}", plain.html);
        let quoted = plain.quoted_html.expect("a quote");
        assert!(quoted.contains("On Mon, 31 Aug 2026"), "{quoted}");
        assert!(quoted.contains("form signed"), "{quoted}");
    }

    #[test]
    fn a_message_with_no_quoting_is_not_split() {
        for name in ["plain-text.eml", "html-hostile.eml", "newsletter-list-unsubscribe.eml"] {
            assert_eq!(rendered(name).quoted_html, None, "{name} was split");
        }
    }

    #[test]
    fn an_invite_parses_to_the_instant_the_organiser_meant() {
        let rendered = rendered("calendar-invite.eml");
        let invite = rendered.invite.expect("an invite");
        // 2026-09-09T17:00:00+05:30, from the VTIMEZONE the sender shipped with it.
        assert_eq!(invite.start_ms, 1_788_953_400_000);
        assert_eq!(invite.end_ms, 1_788_953_400_000 + 45 * 60 * 1000);
        assert!(!invite.all_day);
        assert_eq!(invite.summary, "Piano lesson: Cooper");
        assert_eq!(invite.location.as_deref(), Some("Sunny Day Music, Bandra"));
        assert_eq!(invite.my_response, InviteResponse::NeedsAction);
        assert!(
            rendered.attachments.is_empty(),
            "the card is the affordance, not a chip: {:?}",
            rendered.attachments
        );
    }

    #[test]
    fn a_message_with_no_message_id_still_renders() {
        let rendered = rendered("no-message-id.eml");
        assert_eq!(rendered.message_id, None);
        assert!(rendered.html.contains("Enrolment received"), "{}", rendered.html);
        assert!(rendered.date_ms.is_some());
    }

    #[test]
    fn a_group_address_list_flattens_to_its_members() {
        let rendered = rendered("group-address-list.eml");
        let addresses: Vec<&str> = rendered.to.iter().map(|to| to.address.as_str()).collect();
        assert_eq!(
            addresses,
            vec![
                "pj@73ai.org",
                "hannah.weiss@example.net",
                "caroline@bauhaus-tickets.example"
            ]
        );
        assert_eq!(rendered.cc[0].address, "office@oakridge-school.example");
    }

    #[test]
    fn the_routing_headers_come_through_as_they_were() {
        let newsletter = rendered("newsletter-list-unsubscribe.eml");
        assert_eq!(newsletter.precedence.as_deref(), Some("bulk"));
        assert_eq!(
            newsletter.list_id.as_deref(),
            Some("the-browser.list.thebrowser.example")
        );
        let unsubscribe = newsletter.unsubscribe.expect("an unsubscribe");
        assert!(unsubscribe.one_click);
        assert_eq!(
            unsubscribe.mailto.as_deref(),
            Some("mailto:unsubscribe+91827@mail.thebrowser.example?subject=unsub")
        );
        assert_eq!(
            unsubscribe.url.as_deref(),
            Some("https://thebrowser.example/unsubscribe?u=91827&id=8f3a1c")
        );
        assert_eq!(
            rendered("nested-multipart.eml").auto_submitted.as_deref(),
            Some("auto-generated")
        );
    }

    #[test]
    fn a_folded_references_header_gives_up_its_first_entry() {
        assert_eq!(
            rendered("reply-references-folded.eml").references_first.as_deref(),
            Some("lease-2026-001@meridianproperties.in")
        );
        assert_eq!(
            rendered("header-oddities.eml").subject,
            "The bond market, again"
        );
    }
}
