// The outgoing side: an editor's HTML in, raw RFC 2822 bytes out.
//
// Two things happen here that the composer cannot do for itself. The stylesheet is folded into the
// markup as inline `style` attributes, because a mail client that drops `<style>` is most of them
// and a message that arrives unstyled looks like a message somebody botched. And a plain text
// alternative is generated from the HTML, because a message with no text part is a message some
// readers see as an empty page, and writing the text by hand twice is how the two drift.
//
// What this does not do is decide anything about sending. There is no queue here, no undo window
// and no provider: bytes in, bytes out, so the send pipeline can build a message without owning a
// network connection and the tests can read the result.

use std::io::Write;

use css_inline::CSSInliner;
use mail_builder::MessageBuilder;

use crate::dto::Person;
use crate::sanitize::{decode_entities, scan};

#[derive(Debug, Clone, Default)]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// A part the body refers to by `cid:`, which is how a pasted image travels.
#[derive(Debug, Clone, Default)]
pub struct OutgoingInline {
    /// Without the angle brackets, as it appears after `cid:` in the markup.
    pub content_id: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Outgoing {
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub reply_to: Vec<Person>,
    pub subject: String,
    /// The editor's markup, before the stylesheet is folded in.
    pub html: String,
    /// The editor's stylesheet. Folded into the markup rather than sent as a `<style>` block.
    pub stylesheet: Option<String>,
    /// The `Message-ID` to stamp. Left out only by a caller that does not care what it gets, which
    /// in practice is a test.
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// The thread's `References`, oldest first, with the message being replied to on the end.
    pub references: Vec<String>,
    pub date_ms: Option<i64>,
    pub attachments: Vec<OutgoingAttachment>,
    pub inline: Vec<OutgoingInline>,
}

pub fn build(message: &Outgoing) -> Result<Vec<u8>, String> {
    if message.from.address.is_empty() {
        return Err("a message with no From address cannot be built".to_string());
    }
    let html = inline_stylesheet(&message.html, message.stylesheet.as_deref())?;
    let text = text_alternative(&html);

    let mut builder = MessageBuilder::new()
        .from(address(&message.from))
        .subject(message.subject.clone())
        .text_body(text)
        .html_body(html);

    if !message.to.is_empty() {
        builder = builder.to(addresses(&message.to));
    }
    if !message.cc.is_empty() {
        builder = builder.cc(addresses(&message.cc));
    }
    if !message.bcc.is_empty() {
        builder = builder.bcc(addresses(&message.bcc));
    }
    if !message.reply_to.is_empty() {
        builder = builder.reply_to(addresses(&message.reply_to));
    }
    if let Some(message_id) = &message.message_id {
        builder = builder.message_id(bare(message_id));
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        builder = builder.in_reply_to(bare(in_reply_to));
    }
    if !message.references.is_empty() {
        builder = builder.references(
            message
                .references
                .iter()
                .map(|id| bare(id))
                .collect::<Vec<_>>(),
        );
    }
    if let Some(date_ms) = message.date_ms {
        builder = builder.date(date_ms / 1000);
    }

    for part in &message.inline {
        builder = builder.inline(
            part.mime_type.clone(),
            part.content_id.clone(),
            part.bytes.clone(),
        );
    }
    for part in &message.attachments {
        builder = builder.attachment(
            part.mime_type.clone(),
            part.filename.clone(),
            part.bytes.clone(),
        );
    }

    let mut out = Vec::new();
    builder
        .write_to(&mut out)
        .map_err(|error| format!("the message could not be written: {error}"))?;
    out.flush().ok();
    Ok(out)
}

fn address(person: &Person) -> mail_builder::headers::address::Address<'static> {
    match &person.name {
        Some(name) => (name.clone(), person.address.clone()).into(),
        None => person.address.clone().into(),
    }
}

fn addresses(people: &[Person]) -> mail_builder::headers::address::Address<'static> {
    people.iter().map(address).collect::<Vec<_>>().into()
}

fn bare(message_id: &str) -> String {
    message_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string()
}

fn inline_stylesheet(html: &str, stylesheet: Option<&str>) -> Result<String, String> {
    let Some(stylesheet) = stylesheet.map(str::trim).filter(|css| !css.is_empty()) else {
        return Ok(html.to_string());
    };
    // No resolver and no network: the only stylesheet that gets folded in is the one handed over.
    CSSInliner::options()
        .load_remote_stylesheets(false)
        .keep_style_tags(false)
        .keep_link_tags(false)
        .build()
        .inline_fragment(html, stylesheet)
        .map_err(|error| format!("the stylesheet could not be inlined: {error}"))
}

/// The plain text alternative.
///
/// Block elements end a line, `<br>` ends a line, a link that says something other than where it
/// goes carries its destination in angle brackets after it, and everything else is the text with
/// its character references resolved. Nobody reads this on purpose, but the readers that fall back
/// to it are the ones with no other option.
pub fn text_alternative(html: &str) -> String {
    const BLOCKS: &[&str] = &[
        "address", "blockquote", "div", "dl", "dt", "dd", "h1", "h2", "h3", "h4", "h5", "h6", "hr",
        "li", "ol", "p", "pre", "table", "tr", "ul",
    ];

    let mut out = String::new();
    let mut cursor = 0usize;
    let mut href: Option<String> = None;
    let mut anchor_text = String::new();

    let push = |out: &mut String, anchor: &mut String, in_anchor: bool, text: &str| {
        let text = decode_entities(text);
        let mut collapsed = String::new();
        let mut space = out.ends_with(['\n', ' ']) || out.is_empty();
        for character in text.chars() {
            if character.is_whitespace() {
                if !space {
                    collapsed.push(' ');
                    space = true;
                }
                continue;
            }
            collapsed.push(character);
            space = false;
        }
        if in_anchor {
            anchor.push_str(&collapsed);
        }
        out.push_str(&collapsed);
    };

    for tag in scan(html) {
        if tag.start > cursor {
            push(&mut out, &mut anchor_text, href.is_some(), &html[cursor..tag.start]);
        }
        cursor = tag.end.max(cursor);

        match (tag.name.as_str(), tag.closing) {
            ("br", _) => out.push('\n'),
            ("a", false) => {
                href = tag.attr("href").map(str::to_string);
                anchor_text.clear();
            }
            ("a", true) => {
                if let Some(destination) = href.take() {
                    if !destination.is_empty()
                        && destination.trim() != anchor_text.trim()
                        && !destination.starts_with('#')
                    {
                        out.push_str(&format!(" <{destination}>"));
                    }
                }
            }
            (name, _) if BLOCKS.contains(&name) => {
                if !out.ends_with("\n\n") && !out.is_empty() {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    if cursor < html.len() {
        push(&mut out, &mut anchor_text, href.is_some(), &html[cursor..]);
    }

    let mut text = String::new();
    let mut blank = 0usize;
    for line in out.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        text.push_str(line);
        text.push('\n');
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::{MessageParser, MimeHeaders, PartType};

    fn person(name: &str, address: &str) -> Person {
        Person {
            name: (!name.is_empty()).then(|| name.to_string()),
            address: address.to_string(),
        }
    }

    fn reply() -> Outgoing {
        Outgoing {
            from: person("Priyanshu Jain", "pj@73ai.org"),
            to: vec![person("Dev Patel", "dev.patel@example.org")],
            subject: "Re: Slides from the talk".to_string(),
            html: r#"<p class="lede">Thanks, got them. The <a href="https://example.org/paper">paper</a> first.</p>"#
                .to_string(),
            stylesheet: Some("p.lede { color: #2b2b2b; font-size: 16px }".to_string()),
            message_id: Some("<reply-01@73ai.org>".to_string()),
            in_reply_to: Some("<CAG5dev-slides-03@mail.example.org>".to_string()),
            references: vec![
                "<CAG5dev-slides-01@mail.example.org>".to_string(),
                "<CAG5pj-slides-02@mail.73ai.org>".to_string(),
                "<CAG5dev-slides-03@mail.example.org>".to_string(),
            ],
            date_ms: Some(1_788_000_000_000),
            ..Outgoing::default()
        }
    }

    #[test]
    fn a_message_with_no_sender_is_refused() {
        assert!(build(&Outgoing::default()).is_err());
    }

    #[test]
    fn the_threading_headers_come_out_where_a_reply_needs_them() {
        let raw = build(&reply()).expect("bytes");
        let message = MessageParser::default().parse(&raw).expect("a message");
        assert_eq!(message.message_id(), Some("reply-01@73ai.org"));
        assert_eq!(
            message.in_reply_to().as_text_list().map(|list| list.to_vec()),
            Some(vec!["CAG5dev-slides-03@mail.example.org".into()])
        );
        assert_eq!(
            message.references().as_text_list().map(|list| list.len()),
            Some(3)
        );
        assert_eq!(message.subject(), Some("Re: Slides from the talk"));
    }

    #[test]
    fn the_stylesheet_ends_up_on_the_element_and_not_in_a_style_block() {
        let raw = build(&reply()).expect("bytes");
        let html = String::from_utf8(raw).expect("utf8");
        assert!(html.contains("color: #2b2b2b") || html.contains("color:#2b2b2b"), "{html}");
        assert!(!html.contains("<style"), "{html}");
    }

    #[test]
    fn every_message_carries_a_text_alternative_built_from_the_html() {
        let raw = build(&reply()).expect("bytes");
        let message = MessageParser::default().parse(&raw).expect("a message");
        let text = message
            .text_body
            .iter()
            .filter_map(|id| message.parts.get(*id as usize))
            .find_map(|part| match &part.body {
                PartType::Text(text) => Some(text.to_string()),
                _ => None,
            })
            .expect("a text part");
        assert!(text.contains("Thanks, got them."), "{text}");
        assert!(text.contains("<https://example.org/paper>"), "{text}");
        assert!(!text.contains('<') || text.contains("<https"), "{text}");
    }

    #[test]
    fn an_inline_part_keeps_its_content_id_and_an_attachment_keeps_its_name() {
        let mut message = reply();
        message.html = r#"<p>Here it is</p><img src="cid:shot@73ai.org">"#.to_string();
        message.stylesheet = None;
        message.inline = vec![OutgoingInline {
            content_id: "shot@73ai.org".to_string(),
            mime_type: "image/png".to_string(),
            bytes: b"\x89PNG\r\n\x1a\nrest".to_vec(),
        }];
        message.attachments = vec![OutgoingAttachment {
            filename: "Angebot Küchenarbeitsplatte.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            bytes: b"%PDF-1.4 rest".to_vec(),
        }];

        let raw = build(&message).expect("bytes");
        let parsed = MessageParser::default().parse(&raw).expect("a message");
        let content_ids: Vec<String> = parsed
            .parts
            .iter()
            .filter_map(|part| part.content_id().map(str::to_string))
            .collect();
        assert_eq!(content_ids, vec!["shot@73ai.org".to_string()]);
        let names: Vec<String> = parsed
            .attachments()
            .filter_map(|part| part.attachment_name().map(str::to_string))
            .collect();
        assert!(
            names.contains(&"Angebot Küchenarbeitsplatte.pdf".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn what_is_built_can_be_rendered_back() {
        let raw = build(&reply()).expect("bytes");
        let rendered = crate::mime::render(&raw, &crate::mime::RenderOptions::default())
            .expect("a render");
        assert_eq!(rendered.subject, "Re: Slides from the talk");
        assert_eq!(rendered.from.address, "pj@73ai.org");
        assert_eq!(rendered.to[0].address, "dev.patel@example.org");
        assert!(rendered.html.contains("Thanks, got them."), "{}", rendered.html);
        assert_eq!(
            rendered.references_first.as_deref(),
            Some("CAG5dev-slides-01@mail.example.org")
        );
    }

    /// The multipart boundary is generated per build, so the bytes are not identical twice over.
    /// Everything the boundary separates is, which is the part a caller can depend on.
    #[test]
    fn two_builds_of_the_same_input_differ_only_in_the_boundary() {
        let first = build(&reply()).expect("bytes");
        let second = build(&reply()).expect("bytes");
        assert_ne!(first, second);

        let options = crate::mime::RenderOptions::default();
        let first = crate::mime::render(&first, &options).expect("a render");
        let second = crate::mime::render(&second, &options).expect("a render");
        assert_eq!(first.html, second.html);
        assert_eq!(first.text, second.text);
        assert_eq!(first.subject, second.subject);
    }
}
