// The sanitiser: sender markup in, something safe for an iframe out.
//
// The order of the passes is the design. Quoting is split off the source markup first, because the
// markers a mail client leaves behind (`class="gmail_quote"`, `id="divRplyFwdMsg"`) are exactly the
// attributes the whitelist is about to throw away. Everything else happens inside one `ammonia`
// clean: images, links and trackers are decided in a single attribute filter rather than by a
// second pass over the output, because a second pass is a second parser and two parsers that
// disagree about the same bytes is how filters are defeated.
//
// `scan` below is a tag reader, not a parser, and nothing security-critical rests on it. It exists
// so the image decision can see an `<img>`'s `width`, `height` and `style` at the moment it decides
// about its `src`, which the attribute filter cannot: the filter is called once per attribute and
// has no way to look sideways at its siblings. When `scan` misreads something the decision falls
// through to "drop it", which is the safe end.

pub mod html;
pub mod links;
pub mod quoted;
pub mod surface;
pub mod trackers;

use std::collections::HashMap;

use crate::dto::Tracker;

/// A part the body refers to by `cid:`, keyed elsewhere by its Content-ID with the angle brackets
/// taken off.
#[derive(Debug, Clone)]
pub struct InlinePart {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// The two decisions the caller is allowed to make about a body.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy {
    /// Remote images the caller has already fetched may be inlined. It never means "let the
    /// renderer fetch": nothing in this module opens a socket.
    pub allow_remote_images: bool,
    pub link_cleaning: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Sanitized {
    pub html: String,
    pub trackers: Vec<Tracker>,
    pub blocked_images: u32,
    pub blocked_urls: Vec<String>,
    /// The Content-IDs that were actually inlined, so the caller knows which parts are already on
    /// screen and must not also become an attachment chip.
    pub used_content_ids: Vec<String>,
}

impl Sanitized {
    /// Folds a second half in, for a body that was split into visible and quoted: the banner counts
    /// the whole message, not the part that happens to be on screen.
    pub fn absorb_counts(&mut self, other: &Sanitized) {
        self.trackers.extend(other.trackers.iter().cloned());
        self.blocked_images += other.blocked_images;
        self.blocked_urls.extend(other.blocked_urls.iter().cloned());
        self.used_content_ids
            .extend(other.used_content_ids.iter().cloned());
    }
}

/// The whole sanitiser, for one fragment of sender HTML.
pub fn sanitize(
    source: &str,
    inline_parts: &HashMap<String, InlinePart>,
    remote_images: &HashMap<String, Vec<u8>>,
    policy: Policy,
) -> Result<Sanitized, String> {
    html::clean(source, inline_parts, remote_images, policy)
}

// -------------------------------------------------------------------------------------------
// The tag reader
// -------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Tag {
    /// Lowercased.
    pub name: String,
    pub closing: bool,
    pub self_closing: bool,
    /// Byte offset of the `<`.
    pub start: usize,
    /// Byte offset one past the `>`.
    pub end: usize,
    /// Lowercased names, values with character references already decoded, which is the form the
    /// sanitiser's attribute filter will see them in.
    pub attrs: Vec<(String, String)>,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .map(|value| value.split_ascii_whitespace().any(|item| item == class))
            .unwrap_or(false)
    }
}

/// Elements whose content is raw text rather than markup, so a `<` inside them is not a tag.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

pub fn scan(html: &str) -> Vec<Tag> {
    let bytes = html.as_bytes();
    let mut tags = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"<!--") {
            i = find(bytes, b"-->", i + 4).map(|at| at + 3).unwrap_or(bytes.len());
            continue;
        }
        if matches!(bytes.get(i + 1), Some(b'!') | Some(b'?')) {
            i = find(bytes, b">", i + 2).map(|at| at + 1).unwrap_or(bytes.len());
            continue;
        }

        let closing = bytes.get(i + 1) == Some(&b'/');
        let name_start = if closing { i + 2 } else { i + 1 };
        let mut cursor = name_start;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'-' || bytes[cursor] == b':')
        {
            cursor += 1;
        }
        if cursor == name_start {
            i += 1;
            continue;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..cursor]).to_ascii_lowercase();
        let (attrs, end, self_closing) = read_attrs(bytes, cursor);

        let raw_text = !closing && RAW_TEXT.contains(&name.as_str());
        tags.push(Tag {
            name: name.clone(),
            closing,
            self_closing,
            start: i,
            end,
            attrs,
        });

        i = if raw_text {
            let close = format!("</{name}");
            find_ascii_ci(bytes, close.as_bytes(), end).unwrap_or(bytes.len())
        } else {
            end
        };
    }

    tags
}

fn read_attrs(bytes: &[u8], from: usize) -> (Vec<(String, String)>, usize, bool) {
    let mut attrs = Vec::new();
    let mut i = from;

    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        match bytes.get(i) {
            None => return (attrs, bytes.len(), false),
            Some(b'>') => return (attrs, i + 1, false),
            Some(b'/') if bytes.get(i + 1) == Some(&b'>') => return (attrs, i + 2, true),
            Some(b'/') => {
                i += 1;
                continue;
            }
            _ => {}
        }

        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'>' | b'/')
        {
            i += 1;
        }
        let name = String::from_utf8_lossy(&bytes[name_start..i]).to_ascii_lowercase();

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(quote @ (b'"' | b'\'')) => {
                    let quote = *quote;
                    i += 1;
                    let value_start = i;
                    while i < bytes.len() && bytes[i] != quote {
                        i += 1;
                    }
                    value = decode_entities(&String::from_utf8_lossy(&bytes[value_start..i]));
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                _ => {
                    let value_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = decode_entities(&String::from_utf8_lossy(&bytes[value_start..i]));
                }
            }
        }
        if !name.is_empty() {
            attrs.push((name, value));
        }
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| from + at)
}

fn find_ascii_ci(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|at| from + at)
}

/// The character references a mail client actually writes. The point is not completeness: it is
/// that `&amp;` in a `src` reaches the image decision as the `&` the sanitiser will hand it.
pub fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'&' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            out.push_str(&value[start..i]);
            continue;
        }
        let Some(semicolon) = find(bytes, b";", i + 1).filter(|at| at - i <= 10) else {
            out.push('&');
            i += 1;
            continue;
        };
        let name = &value[i + 1..semicolon];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => name
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                i = semicolon + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

/// The visible text of a fragment, with the markup and the character references taken out and the
/// whitespace collapsed. What the snippet and the search index are made of.
pub fn text_of(html: &str) -> String {
    let tags = scan(html);
    let mut out = String::new();
    let mut cursor = 0usize;

    let push_text = |out: &mut String, slice: &str| {
        let decoded = decode_entities(slice);
        for word in decoded.split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    };

    for tag in &tags {
        if tag.start > cursor {
            push_text(&mut out, &html[cursor..tag.start]);
        }
        cursor = tag.end.max(cursor);
    }
    if cursor < html.len() {
        push_text(&mut out, &html[cursor..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_attributes_in_every_quoting_style() {
        let tags = scan(r#"<img src='a.png' width=1 alt="x y" hidden>"#);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].attr("src"), Some("a.png"));
        assert_eq!(tags[0].attr("width"), Some("1"));
        assert_eq!(tags[0].attr("alt"), Some("x y"));
        assert_eq!(tags[0].attr("hidden"), Some(""));
    }

    #[test]
    fn decodes_references_in_attribute_values() {
        let tags = scan(r#"<img src="https://h/p?a=1&amp;b=2&#38;c=3">"#);
        assert_eq!(tags[0].attr("src"), Some("https://h/p?a=1&b=2&c=3"));
    }

    #[test]
    fn a_less_than_inside_a_script_is_not_a_tag() {
        let tags = scan("<script>if (a<b) { }</script><p>x</p>");
        let names: Vec<_> = tags.iter().map(|tag| tag.name.as_str()).collect();
        assert_eq!(names, vec!["script", "script", "p", "p"]);
    }

    #[test]
    fn skips_comments_and_doctypes() {
        let tags = scan("<!DOCTYPE html><!-- <img src=x> --><b>hi</b>");
        let names: Vec<_> = tags.iter().map(|tag| tag.name.as_str()).collect();
        assert_eq!(names, vec!["b", "b"]);
    }

    #[test]
    fn multi_byte_text_does_not_split_a_tag() {
        let tags = scan("<p>Inês café, ok</p><img src=\"x\">");
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[2].attr("src"), Some("x"));
    }

    #[test]
    fn text_of_collapses_whitespace() {
        assert_eq!(text_of("<p>a\n  b</p><p>c&amp;d</p>"), "a b c&d");
    }
}
