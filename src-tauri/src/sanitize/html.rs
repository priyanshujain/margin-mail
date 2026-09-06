// The whitelist. This file is the answer to "what can a message do to me".
//
// It starts from `ammonia`'s default policy rather than a hand written tag list, because the
// default already drops `script`, `style`, `iframe`, `form`, `object`, `embed`, `meta`, `link`,
// every `on*` handler and every scheme that is not on its list, and a list typed out by hand here
// would be that list minus whatever was forgotten. What follows is only the difference: the tags
// and attributes mail needs that a web page does not, and the three decisions `ammonia` has no
// opinion about.
//
// 1. Images. Every `img src` goes through `plan_images` first and the attribute filter second, and
//    the filter drops anything the plan did not account for. A `cid:` reference becomes a `data:`
//    URI built from the part's own bytes. A remote URL is dropped and counted, or, when the caller
//    has already fetched it and handed the bytes in, inlined the same way. No `http` or `https`
//    URL reaches the output in an `img src` under any setting, which is the whole privacy claim:
//    opening a message cannot be observed because opening a message fetches nothing.
// 2. Style. `style` is allowed as an attribute and then cut down to the properties in
//    `STYLE_PROPERTIES`. Nothing on that list can take a `url()`, so a style attribute cannot
//    become a fetch, and nothing on it can take an element out of its box, so a message cannot
//    draw over the app's own chrome. A declaration block that mentions `url(` at all is dropped
//    whole before the property filter runs, because the property filter is a CSS parser and the
//    obfuscations that get past a CSS parser are the ones written to get past a CSS parser.
// 3. Links. Scheme and tracking parameters in `links`, then `target="_blank"` and
//    `rel="noopener noreferrer"` on every anchor.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ammonia::Builder;
use base64::Engine;

use super::{decode_entities, escape_html, links, scan, trackers, InlinePart, Policy, Sanitized, Tag};
use crate::dto::Tracker;

/// Properties a sender may set. Colour, type, spacing, alignment, borders and table layout: the
/// things that make a newsletter look like itself. Everything that can carry a `url()`
/// (`background`, `background-image`, `list-style-image`, `border-image`, `cursor`, `content`,
/// `filter`, `src`) and everything that positions or hides (`position`, `top`, `left`, `right`,
/// `bottom`, `z-index`, `float`, `clip`, `transform`, `display`, `visibility`, `opacity`,
/// `overflow`) is absent, and absent is the decision rather than an oversight. `display` costs a
/// newsletter its hidden preheader line, which is a fair price for a message being unable to hide
/// text from the person reading it.
const STYLE_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "word-break",
    "overflow-wrap",
    "text-align",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-style",
    "text-indent",
    "text-transform",
    "vertical-align",
    "white-space",
    "direction",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-top-color",
    "border-right-color",
    "border-bottom-color",
    "border-left-color",
    "border-style",
    "border-top-style",
    "border-right-style",
    "border-bottom-style",
    "border-left-style",
    "border-width",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "border-radius",
    "border-collapse",
    "border-spacing",
    "width",
    "min-width",
    "max-width",
    "height",
    "min-height",
    "max-height",
    "table-layout",
    "caption-side",
    "empty-cells",
    "list-style-type",
    "list-style-position",
];

/// Elements whose text goes with them. `ammonia` unwraps a tag it does not allow and keeps what was
/// inside, which is right for a `<span>` and wrong for a `<form>`: the submit button's label is not
/// something the sender wrote to be read, and left behind on its own it reads as a stray line the
/// message did not mean to say. `script` and `style` are already on the default list.
const EXTRA_CLEAN_CONTENT_TAGS: &[&str] = &[
    "applet", "audio", "button", "embed", "form", "frame", "frameset", "iframe", "input", "math",
    "noscript", "object", "option", "select", "svg", "template", "textarea", "title", "video",
];

/// Tags mail uses that a web page has stopped using. `tfoot` is in `ammonia`'s attribute table but
/// not its tag list, which would drop the footer row of every invoice.
const EXTRA_TAGS: &[&str] = &["tfoot", "font", "address"];

/// `style` because of `STYLE_PROPERTIES` above, `dir` because right to left mail is mail.
const EXTRA_GENERIC_ATTRIBUTES: &[&str] = &["style", "dir"];

/// The presentational attributes a table based newsletter is built out of. `background` is not
/// here: it is a URL in an attribute, which is the same fetch by another name.
const EXTRA_TAG_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("table", &["bgcolor", "border", "cellpadding", "cellspacing", "width", "height"]),
    ("thead", &["bgcolor", "valign"]),
    ("tbody", &["bgcolor", "valign"]),
    ("tfoot", &["align", "bgcolor", "valign"]),
    ("tr", &["bgcolor", "height", "valign"]),
    ("td", &["bgcolor", "height", "nowrap", "valign", "width"]),
    ("th", &["bgcolor", "height", "nowrap", "valign", "width"]),
    ("font", &["color", "face", "size"]),
    ("img", &["border", "hspace", "vspace"]),
    ("a", &["name"]),
];

/// Schemes an attribute value may still hold when the attribute filter is asked about it. `cid`
/// and `data` are here so the filter gets the chance to decide: `ammonia` drops an unlisted scheme
/// while parsing, which is before the filter runs, and a dropped `cid:` cannot be inlined. Both are
/// refused for `href` in `links::safe_href`.
const URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel", "sms", "webcal", "geo", "cid", "data"];

/// Image types that can be shown without running anything. `image/svg+xml` is a document, not a
/// picture, so it is not on the list.
const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp"];

#[derive(Debug, Clone)]
enum ImagePlan {
    Inline {
        data_url: String,
        content_id: Option<String>,
    },
    Tracker(Tracker),
    Blocked(String),
}

#[derive(Debug, Clone)]
enum ImageEvent {
    Inlined(Option<String>),
    Tracked(Tracker),
    Blocked(String),
}

pub fn clean(
    source: &str,
    inline_parts: &HashMap<String, InlinePart>,
    remote_images: &HashMap<String, Vec<u8>>,
    policy: Policy,
) -> Result<Sanitized, String> {
    let plans = plan_images(source, inline_parts, remote_images, policy);
    let log: Arc<Mutex<Vec<ImageEvent>>> = Arc::new(Mutex::new(Vec::new()));

    let filter_log = Arc::clone(&log);
    let link_cleaning = policy.link_cleaning;
    let mut builder = Builder::default();
    builder
        .add_tags(EXTRA_TAGS)
        .add_clean_content_tags(EXTRA_CLEAN_CONTENT_TAGS.iter().copied())
        .add_generic_attributes(EXTRA_GENERIC_ATTRIBUTES)
        .url_schemes(URL_SCHEMES.iter().copied().collect())
        .filter_style_properties(STYLE_PROPERTIES.iter().copied().collect())
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(move |element, attribute, value| {
            filter(&plans, &filter_log, link_cleaning, element, attribute, value)
        });
    for (tag, attributes) in EXTRA_TAG_ATTRIBUTES {
        builder.add_tag_attributes(tag, attributes.iter().copied());
    }

    let html = builder.clean(source).to_string();

    let events = log
        .lock()
        .map_err(|_| "the image log was poisoned".to_string())?;
    let mut sanitized = Sanitized {
        html,
        ..Sanitized::default()
    };
    for event in events.iter() {
        match event {
            ImageEvent::Inlined(Some(content_id)) => {
                sanitized.used_content_ids.push(content_id.clone())
            }
            ImageEvent::Inlined(None) => {}
            ImageEvent::Tracked(tracker) => sanitized.trackers.push(tracker.clone()),
            ImageEvent::Blocked(url) => {
                sanitized.blocked_images += 1;
                sanitized.blocked_urls.push(url.clone());
            }
        }
    }
    Ok(sanitized)
}

fn filter<'u>(
    plans: &HashMap<String, ImagePlan>,
    log: &Mutex<Vec<ImageEvent>>,
    link_cleaning: bool,
    element: &str,
    attribute: &str,
    value: &'u str,
) -> Option<Cow<'u, str>> {
    let record = |event: ImageEvent| {
        if let Ok(mut log) = log.lock() {
            log.push(event);
        }
    };

    match (element, attribute) {
        ("img", "src") => match plans.get(value.trim()) {
            Some(ImagePlan::Inline {
                data_url,
                content_id,
            }) => {
                record(ImageEvent::Inlined(content_id.clone()));
                Some(Cow::Owned(data_url.clone()))
            }
            Some(ImagePlan::Tracker(tracker)) => {
                record(ImageEvent::Tracked(tracker.clone()));
                None
            }
            Some(ImagePlan::Blocked(url)) => {
                record(ImageEvent::Blocked(url.clone()));
                None
            }
            // Nothing planned for it, so nothing loads it. Every path out of `plan_images` that
            // does not end in a plan is one that decided the image should not be shown.
            None => None,
        },
        // No other element gets to name a resource. `src` on anything else in the allowed tag set
        // would be a fetch with no user visible box to justify it.
        (_, "src") => None,
        // `cite` is not fetched and not navigated to by any browser, but it is a URL in the
        // output and the rule is that a URL in the output has been through the same door.
        (_, "href") | (_, "cite") => links::safe_href(value, link_cleaning).map(Cow::Owned),
        (_, "style") => style_is_safe(value).then(|| Cow::Borrowed(value)),
        // Belt and braces: `on*` is not in the whitelist, so this arm should be unreachable. It
        // costs one comparison and it means a future widening of the attribute list cannot open a
        // handler by accident.
        _ if attribute.starts_with("on") => None,
        _ => Some(Cow::Borrowed(value)),
    }
}

/// What happens to each `img src` in the fragment, decided with the whole tag in view.
///
/// The attribute filter cannot do this itself: it is handed one attribute at a time with no way to
/// see whether the same tag also carried `width="1"` or `style="display:none"`, and those are half
/// of what makes an image a tracker.
fn plan_images(
    source: &str,
    inline_parts: &HashMap<String, InlinePart>,
    remote_images: &HashMap<String, Vec<u8>>,
    policy: Policy,
) -> HashMap<String, ImagePlan> {
    let mut plans = HashMap::new();
    for tag in scan(source) {
        if tag.closing || tag.name != "img" {
            continue;
        }
        let Some(src) = tag.attr("src").map(str::trim).filter(|src| !src.is_empty()) else {
            continue;
        };
        if let Some(plan) = plan_one(src, &tag, inline_parts, remote_images, policy) {
            plans.insert(src.to_string(), plan);
        }
    }
    plans
}

fn plan_one(
    src: &str,
    tag: &Tag,
    inline_parts: &HashMap<String, InlinePart>,
    remote_images: &HashMap<String, Vec<u8>>,
    policy: Policy,
) -> Option<ImagePlan> {
    let lowercase = src.to_ascii_lowercase();

    if let Some(reference) = lowercase.strip_prefix("cid:") {
        let key = decode_entities(reference)
            .trim_matches(|character| character == '<' || character == '>')
            .to_string();
        let part = inline_parts.get(&key)?;
        let mime_type = part.mime_type.to_ascii_lowercase();
        if !IMAGE_TYPES.contains(&mime_type.as_str()) {
            return None;
        }
        return Some(ImagePlan::Inline {
            data_url: data_url(&mime_type, &part.bytes),
            content_id: Some(key),
        });
    }

    if lowercase.starts_with("data:") {
        let mime_type = lowercase
            .trim_start_matches("data:")
            .split(&[';', ','][..])
            .next()
            .unwrap_or_default()
            .to_string();
        return IMAGE_TYPES.contains(&mime_type.as_str()).then(|| ImagePlan::Inline {
            data_url: src.to_string(),
            content_id: None,
        });
    }

    if !lowercase.starts_with("http://") && !lowercase.starts_with("https://") {
        return None;
    }

    // A tracker stays removed even when the reader has asked for images: "show images" is a request
    // to see the pictures, not a request to file an open report.
    if let Some(vendor) = trackers::classify(src, tag) {
        return Some(ImagePlan::Tracker(Tracker {
            vendor,
            url: src.to_string(),
        }));
    }

    if policy.allow_remote_images {
        if let Some(bytes) = remote_images.get(src) {
            if let Some(mime_type) = sniff_image_type(bytes) {
                return Some(ImagePlan::Inline {
                    data_url: data_url(mime_type, bytes),
                    content_id: None,
                });
            }
        }
    }
    Some(ImagePlan::Blocked(src.to_string()))
}

fn data_url(mime_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The type of the bytes, not the type the URL claimed. A caller that fetched a URL got whatever
/// the server felt like sending, and the sniff is the only statement about it worth trusting.
fn sniff_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    None
}

/// True when a style attribute cannot become a fetch.
///
/// The check runs on the declaration block with whitespace and comments taken out, because
/// `url ( x )` and `u/**/rl(x)` are the same declaration to a browser and three different strings
/// to a naive `contains`. A backslash is refused outright: in CSS it starts an escape, and an
/// escape is how `\75 rl(` is written by someone who read this function.
fn style_is_safe(value: &str) -> bool {
    let mut normalised = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            i = match bytes[i + 2..].windows(2).position(|window| window == b"*/") {
                Some(at) => i + 2 + at + 2,
                None => bytes.len(),
            };
            continue;
        }
        if !bytes[i].is_ascii_whitespace() {
            normalised.push(bytes[i].to_ascii_lowercase() as char);
        }
        i += 1;
    }
    !normalised.contains('\\')
        && !normalised.contains("url(")
        && !normalised.contains("image(")
        && !normalised.contains("image-set(")
        && !normalised.contains("expression(")
        && !normalised.contains("@import")
}

/// A plain text body, as HTML.
///
/// Escape, keep the paragraphs, turn bare URLs into links, and stop. The pane sets the result in
/// the text face on a 46em measure, so the sender's own wrapping is not worth preserving where it
/// was clearly the sender's mail client wrapping at 72 columns. The rule for telling those apart is
/// the one every mail client uses: a line long enough to have been wrapped is joined to the next,
/// a short line kept its break because someone pressed return. It gets a signature block right and
/// a hard wrapped paragraph right, which no single rule does.
pub fn from_text(text: &str) -> String {
    const WRAPPED_AT: usize = 60;

    let mut out = String::new();
    for block in text.replace("\r\n", "\n").replace('\r', "\n").split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.iter().all(|line| line.trim().is_empty()) {
            continue;
        }
        let mut paragraph = String::new();
        for (index, line) in lines.iter().enumerate() {
            paragraph.push_str(&linkify(line.trim_end()));
            let Some(next) = lines.get(index + 1) else {
                continue;
            };
            let joins = line.chars().count() >= WRAPPED_AT
                && !next.trim().is_empty()
                && !next.starts_with([' ', '\t'])
                && !starts_a_list_item(next);
            paragraph.push_str(if joins { " " } else { "<br>\n" });
        }
        out.push_str("<p>");
        out.push_str(&paragraph);
        out.push_str("</p>\n");
    }
    out
}

fn starts_a_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("• ")
        || trimmed.starts_with('>')
        || trimmed
            .split_once(['.', ')'])
            .map(|(head, _)| !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

/// Bare `http` and `https` URLs become links. Everything else in the line is escaped text: the
/// output of this function goes through the sanitiser like any other markup, so a mistake here is
/// an ugly line rather than a hole.
fn linkify(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut plain_from = 0usize;

    while i < bytes.len() {
        let is_start = (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            && (bytes[i..].starts_with(b"http://") || bytes[i..].starts_with(b"https://"));
        if !is_start {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < bytes.len()
            && !bytes[end].is_ascii_whitespace()
            && !matches!(bytes[end], b'<' | b'>' | b'"')
        {
            end += 1;
        }
        while end > i && matches!(bytes[end - 1], b'.' | b',' | b';' | b':' | b')' | b']' | b'"' | b'\'') {
            end -= 1;
        }
        out.push_str(&escape_html(&line[plain_from..i]));
        let url = escape_html(&line[i..end]);
        out.push_str(&format!(r#"<a href="{url}">{url}</a>"#));
        i = end;
        plain_from = end;
    }
    out.push_str(&escape_html(&line[plain_from..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_default(source: &str) -> Sanitized {
        clean(
            source,
            &HashMap::new(),
            &HashMap::new(),
            Policy {
                allow_remote_images: false,
                link_cleaning: true,
            },
        )
        .expect("a clean")
    }

    #[test]
    fn the_dangerous_elements_do_not_survive() {
        let out = clean_default(
            r#"<script>x=1</script><iframe src="https://e.example"></iframe>
               <form action="https://e.example"><input name="a"></form>
               <object data="https://e.example/x.swf"></object><embed src="x.swf">
               <meta http-equiv="refresh" content="0;url=https://e.example">
               <p onclick="alert(1)">text</p>"#,
        );
        for forbidden in ["<script", "<iframe", "<form", "<object", "<embed", "<meta", "onclick"] {
            assert!(!out.html.contains(forbidden), "{forbidden} survived: {}", out.html);
        }
        assert!(out.html.contains("text"));
    }

    #[test]
    fn an_anchor_leaves_with_a_target_and_a_rel() {
        let out = clean_default(r#"<a href="https://e.example/a">go</a>"#);
        assert!(out.html.contains(r#"target="_blank""#), "{}", out.html);
        assert!(out.html.contains(r#"rel="noopener noreferrer""#), "{}", out.html);
    }

    #[test]
    fn code_hrefs_are_dropped_and_the_text_is_kept() {
        let out = clean_default(r#"<a href="javascript:alert(1)">book now</a>"#);
        assert!(!out.html.contains("javascript"), "{}", out.html);
        assert!(out.html.contains("book now"));
    }

    #[test]
    fn a_style_that_can_fetch_is_dropped_whole() {
        for style in [
            "background-image: url('https://e.example/b.png')",
            "background-image: u/**/rl(https://e.example/b.png)",
            r"background-image: \75 rl(https://e.example/b.png)",
            "background: url ( https://e.example/b.png )",
        ] {
            let out = clean_default(&format!(r#"<p style="{style}">x</p>"#));
            assert!(!out.html.contains("url"), "{style} survived as {}", out.html);
            assert!(!out.html.contains("e.example"), "{style} survived as {}", out.html);
        }
    }

    #[test]
    fn a_style_that_only_paints_is_kept_and_trimmed() {
        let out = clean_default(r#"<p style="color:red;position:absolute;top:0">x</p>"#);
        assert!(out.html.contains("color"), "{}", out.html);
        assert!(!out.html.contains("position"), "{}", out.html);
        assert!(!out.html.contains("top"), "{}", out.html);
    }

    #[test]
    fn a_remote_image_is_blocked_and_counted() {
        let out = clean_default(r#"<img src="https://e.example/hero.png" width="600" height="200">"#);
        assert_eq!(out.blocked_images, 1);
        assert_eq!(out.blocked_urls, vec!["https://e.example/hero.png".to_string()]);
        assert!(!out.html.contains("e.example"), "{}", out.html);
    }

    #[test]
    fn an_ampersand_in_a_src_does_not_lose_the_image_its_plan() {
        let out = clean_default(r#"<img src="https://e.example/p.gif?a=1&amp;b=2" width="1" height="1">"#);
        assert_eq!(out.trackers.len(), 1, "{out:?}");
        assert_eq!(out.trackers[0].url, "https://e.example/p.gif?a=1&b=2");
    }

    #[test]
    fn a_cid_reference_becomes_the_parts_own_bytes() {
        let png = b"\x89PNG\r\n\x1a\n rest".to_vec();
        let mut parts = HashMap::new();
        parts.insert(
            "photo@example.net".to_string(),
            InlinePart {
                mime_type: "image/png".to_string(),
                bytes: png.clone(),
            },
        );
        let out = clean(
            r#"<img src="cid:photo@example.net" alt="a">"#,
            &parts,
            &HashMap::new(),
            Policy::default(),
        )
        .expect("a clean");
        assert!(out.html.contains("data:image/png;base64,"), "{}", out.html);
        assert!(!out.html.contains("cid:"), "{}", out.html);
        assert_eq!(out.used_content_ids, vec!["photo@example.net".to_string()]);
        assert_eq!(out.blocked_images, 0);
    }

    #[test]
    fn a_fetched_remote_image_is_inlined_only_when_it_is_really_an_image() {
        let mut remote = HashMap::new();
        remote.insert(
            "https://e.example/hero.png".to_string(),
            b"\x89PNG\r\n\x1a\n rest".to_vec(),
        );
        remote.insert("https://e.example/not.png".to_string(), b"<html>".to_vec());
        let policy = Policy {
            allow_remote_images: true,
            link_cleaning: false,
        };
        let out = clean(
            r#"<img src="https://e.example/hero.png"><img src="https://e.example/not.png">"#,
            &HashMap::new(),
            &remote,
            policy,
        )
        .expect("a clean");
        assert!(out.html.contains("data:image/png;base64,"), "{}", out.html);
        assert!(!out.html.contains("https://"), "{}", out.html);
        assert_eq!(out.blocked_images, 1);
    }

    #[test]
    fn showing_images_does_not_show_trackers() {
        let mut remote = HashMap::new();
        remote.insert(
            "https://track.hubspot.com/__ptq.gif".to_string(),
            b"GIF89a rest".to_vec(),
        );
        let out = clean(
            r#"<img src="https://track.hubspot.com/__ptq.gif" width="1" height="1">"#,
            &HashMap::new(),
            &remote,
            Policy {
                allow_remote_images: true,
                link_cleaning: false,
            },
        )
        .expect("a clean");
        assert_eq!(out.trackers.len(), 1);
        assert_eq!(out.trackers[0].vendor, "HubSpot");
        assert!(!out.html.contains("data:"), "{}", out.html);
    }

    #[test]
    fn the_other_ways_to_name_a_remote_resource_do_not_work_either() {
        for markup in [
            r#"<img srcset="https://e.example/a.png 1x" src="https://e.example/a.png">"#,
            r#"<picture><source srcset="https://e.example/a.png"><img alt="x"></picture>"#,
            r#"<td background="https://e.example/b.png">x</td>"#,
            r#"<table background="https://e.example/b.png"><tr><td>x</td></tr></table>"#,
            r#"<input type="image" src="https://e.example/a.png">"#,
            r#"<image src="https://e.example/a.png">"#,
            r#"<svg><image href="https://e.example/a.png"></svg>"#,
            r#"<video poster="https://e.example/a.png"><source src="https://e.example/a.mp4"></video>"#,
            r#"<a href="https://e.example/a" ping="https://e.example/track">go</a>"#,
        ] {
            let out = clean_default(markup);
            assert!(
                !out.html.contains("e.example/a.png")
                    && !out.html.contains("e.example/b.png")
                    && !out.html.contains("e.example/a.mp4")
                    && !out.html.contains("e.example/track"),
                "{markup} survived as {}",
                out.html
            );
        }
    }

    #[test]
    fn a_sender_supplied_svg_data_url_is_not_an_image() {
        let out = clean_default(r#"<img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=">"#);
        assert!(!out.html.contains("data:"), "{}", out.html);
    }

    #[test]
    fn plain_text_keeps_paragraphs_and_rejoins_wrapped_lines() {
        let html = from_text(
            "Priya said the place on Church Street takes bookings now, want me to put\r\nus down for four at eight?\r\n\r\nMaya\r\nSecond line\r\n",
        );
        assert!(html.contains("want me to put us down"), "{html}");
        assert!(html.contains("Maya<br>\nSecond line"), "{html}");
        assert_eq!(html.matches("<p>").count(), 2, "{html}");
    }

    #[test]
    fn plain_text_is_escaped_and_bare_urls_become_links() {
        let html = from_text("see <https://e.example/a?b=1&c=2> and \"quotes\"\n");
        assert!(html.contains("&lt;"), "{html}");
        assert!(html.contains(r#"<a href="https://e.example/a?b=1&amp;c=2">"#), "{html}");
        assert!(!html.contains("<script"), "{html}");
    }
}
