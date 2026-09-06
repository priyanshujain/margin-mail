// Where the new message stops and the conversation underneath it starts.
//
// There is no standard for this and there never was, so the only thing to do is to know the shapes
// the clients people actually use leave behind: Gmail's `gmail_quote` container, Outlook's
// `divRplyFwdMsg` and the rule above it, Apple Mail and Thunderbird's trailing `blockquote`, the
// `On <date>, <person> wrote:` attribution over a run of `>` lines, and the
// `-----Original Message-----` divider.
//
// Every rule here carries the same guard: the candidate is only a split point when everything after
// it is the quote, and when something is left over above it. Failing that guard means no split, and
// no split means the reader sees the whole message with nothing hidden. That is the direction to be
// wrong in. Hiding a paragraph the sender wrote is a bug the reader cannot see; showing a quote
// they have read before is a bug they can.
//
// The split runs on the source markup rather than the sanitiser's output, because `class` and `id`
// are how a quote announces itself and the whitelist is about to throw both away.

use super::{scan, text_of, Tag};

/// The visible body, and the quoted conversation when there is one to take off.
pub fn split_html(html: &str) -> (String, Option<String>) {
    let tags = scan(html);
    let mut best: Option<usize> = None;

    for (index, tag) in tags.iter().enumerate() {
        if tag.closing {
            continue;
        }
        let Some(kind) = quote_container(tag) else {
            continue;
        };
        let end = element_end(html, &tags, index);
        if !is_trailing(&html[end..]) {
            continue;
        }
        let cut = match kind {
            // Outlook draws a rule above the forwarded header block. Cutting below it leaves a bare
            // horizontal line at the bottom of every reply.
            Container::ReplyForwardHeader => rule_above(html, &tags, index).unwrap_or(tag.start),
            Container::Other => tag.start,
        };
        if !has_visible_text(&html[..cut]) {
            continue;
        }
        best = Some(best.map_or(cut, |current: usize| current.min(cut)));
    }

    if let Some(cut) = text_divider(html, &tags) {
        if has_visible_text(&html[..cut]) {
            best = Some(best.map_or(cut, |current: usize| current.min(cut)));
        }
    }

    match best.map(|cut| back_over_breaks(html, &tags, cut)) {
        Some(cut) => (html[..cut].to_string(), Some(html[cut..].to_string())),
        None => (html.to_string(), None),
    }
}

/// The line breaks a client leaves between the reply and the quote belong to the quote. Left on the
/// visible side they are a blank line under the last thing the sender wrote.
fn back_over_breaks(html: &str, tags: &[Tag], cut: usize) -> usize {
    let mut cut = cut;
    loop {
        let Some(previous) = tags.iter().rev().find(|tag| tag.end <= cut) else {
            return cut;
        };
        if previous.name != "br" || !html[previous.end..cut].trim().is_empty() {
            return cut;
        }
        cut = previous.start;
    }
}

enum Container {
    ReplyForwardHeader,
    Other,
}

fn quote_container(tag: &Tag) -> Option<Container> {
    let id = tag.attr("id").unwrap_or_default().to_ascii_lowercase();
    if id == "divrplyfwdmsg" || id.starts_with("yahoo_quoted") {
        return Some(Container::ReplyForwardHeader);
    }
    if tag.name == "blockquote" {
        return Some(Container::Other);
    }
    for class in ["gmail_quote", "gmail_quote_container", "yahoo_quoted", "moz-cite-prefix"] {
        if tag.has_class(class) {
            return Some(Container::Other);
        }
    }
    None
}

/// The byte just past the element's own closing tag, or the end of the fragment when the sender
/// never closed it.
fn element_end(html: &str, tags: &[Tag], index: usize) -> usize {
    let tag = &tags[index];
    if tag.self_closing || matches!(tag.name.as_str(), "br" | "hr" | "img" | "wbr") {
        return tag.end;
    }
    let mut depth = 0usize;
    for candidate in &tags[index..] {
        if candidate.name != tag.name {
            continue;
        }
        if candidate.closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return candidate.end;
            }
        } else if !candidate.self_closing {
            depth += 1;
        }
    }
    html.len()
}

/// True when what is left is only the ancestors closing themselves.
fn is_trailing(rest: &str) -> bool {
    for tag in scan(rest) {
        if !tag.closing && !matches!(tag.name.as_str(), "br" | "wbr" | "hr") {
            return false;
        }
    }
    text_of(rest).trim().is_empty()
}

fn has_visible_text(html: &str) -> bool {
    !text_of(html).trim().is_empty()
}

/// An Outlook style rule sitting immediately above the forwarded header block.
fn rule_above(html: &str, tags: &[Tag], index: usize) -> Option<usize> {
    let previous = tags[..index].iter().rev().find(|tag| tag.name != "br")?;
    if previous.name != "hr" || previous.closing {
        return None;
    }
    html[previous.end..tags[index].start]
        .trim()
        .is_empty()
        .then_some(previous.start)
}

const TEXT_DIVIDERS: &[&str] = &[
    "-----Original Message-----",
    "-----Forwarded message-----",
    "---------- Forwarded message ---------",
];

/// A divider written as text rather than as markup. The cut goes to the start of the element that
/// holds it, so the divider itself lands on the quoted side.
fn text_divider(html: &str, tags: &[Tag]) -> Option<usize> {
    let lowercase = html.to_ascii_lowercase();
    let at = TEXT_DIVIDERS
        .iter()
        .filter_map(|divider| lowercase.find(&divider.to_ascii_lowercase()))
        .min()?;
    Some(
        tags.iter()
            .rev()
            .find(|tag| tag.start < at)
            .map(|tag| tag.start)
            .unwrap_or(at),
    )
}

/// The same job on a plain text body, before it is turned into HTML.
pub fn split_text(text: &str) -> (String, Option<String>) {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalised.split('\n').collect();

    let cut = (0..lines.len()).find(|index| {
        let index = *index;
        // Something of the sender's own has to be left above the cut, and a line that is already
        // quoted is not the sender's own.
        if !lines[..index]
            .iter()
            .any(|line| !line.trim().is_empty() && !line.starts_with('>'))
        {
            return false;
        }
        if lines[index].trim().is_empty() {
            return false;
        }
        divider_line(lines[index])
            || attribution_over_a_quote(&lines, index)
            || quote_run_start(&lines, index)
    });

    let Some(cut) = cut else {
        return (normalised, None);
    };
    let visible = lines[..cut].join("\n").trim_end().to_string();
    if visible.is_empty() {
        return (normalised, None);
    }
    (visible, Some(lines[cut..].join("\n")))
}

fn divider_line(line: &str) -> bool {
    let trimmed = line.trim();
    TEXT_DIVIDERS
        .iter()
        .any(|divider| trimmed.eq_ignore_ascii_case(divider))
}

/// `On <date>, <person> wrote:` over lines that are all quoted. The attribution is allowed to wrap,
/// because every client wraps it somewhere different.
fn attribution_over_a_quote(lines: &[&str], index: usize) -> bool {
    for span in 1..=3usize {
        let Some(window) = lines.get(index..index + span) else {
            break;
        };
        let joined = window.join(" ");
        let joined = joined.trim();
        if !joined.starts_with("On ") || !joined.ends_with("wrote:") {
            continue;
        }
        if everything_after_is_quoted(lines, index + span) {
            return true;
        }
    }
    false
}

fn quote_run_start(lines: &[&str], index: usize) -> bool {
    lines[index].starts_with('>') && everything_after_is_quoted(lines, index)
}

fn everything_after_is_quoted(lines: &[&str], from: usize) -> bool {
    let rest = &lines[from..];
    rest.iter().any(|line| line.starts_with('>'))
        && rest
            .iter()
            .all(|line| line.trim().is_empty() || line.starts_with('>'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gmail_container_at_the_end_is_the_split_point() {
        let html = r#"<div dir="ltr">Here they are.</div><br><div class="gmail_quote"><div class="gmail_attr">On Mon, Priyanshu wrote:</div><blockquote class="gmail_quote">Send the slides?</blockquote></div>"#;
        let (visible, quoted) = split_html(html);
        assert!(visible.contains("Here they are"));
        assert!(!visible.contains("Send the slides"));
        assert!(quoted.expect("a quote").contains("Send the slides"));
    }

    #[test]
    fn a_trailing_blockquote_is_the_split_point() {
        let html = "<p>Yes, Thursday works.</p><blockquote type=\"cite\"><p>Are you free Thursday?</p></blockquote>";
        let (visible, quoted) = split_html(html);
        assert_eq!(visible, "<p>Yes, Thursday works.</p>");
        assert!(quoted.expect("a quote").starts_with("<blockquote"));
    }

    #[test]
    fn a_blockquote_in_the_middle_is_not_a_split_point() {
        let html = "<p>She wrote:</p><blockquote><p>Bring a hat.</p></blockquote><p>So I did.</p>";
        assert_eq!(split_html(html), (html.to_string(), None));
    }

    #[test]
    fn a_message_that_is_only_a_quote_is_not_split() {
        let html = "<blockquote><p>Bring a hat.</p></blockquote>";
        assert_eq!(split_html(html), (html.to_string(), None));
    }

    #[test]
    fn an_outlook_rule_goes_with_the_quote_it_introduces() {
        let html = r#"<p>Sending it on.</p><hr><div id="divRplyFwdMsg"><b>From:</b> Sam</div>"#;
        let (visible, quoted) = split_html(html);
        assert_eq!(visible, "<p>Sending it on.</p>");
        assert!(quoted.expect("a quote").starts_with("<hr>"));
    }

    #[test]
    fn a_written_out_divider_splits_too() {
        let html = "<p>See below.</p><p>-----Original Message-----<br>From: Sam</p>";
        let (visible, quoted) = split_html(html);
        assert_eq!(visible, "<p>See below.</p>");
        assert!(quoted.expect("a quote").contains("Original Message"));
    }

    #[test]
    fn an_attribution_over_quoted_lines_splits_plain_text() {
        let text = "Got it, thank you.\n\nSam\n\nOn Mon, 31 Aug 2026 at 09:12, Priyanshu Jain <pj@73ai.org> wrote:\n> Would a weekday suit?\n>\n> Thanks\n";
        let (visible, quoted) = split_text(text);
        assert_eq!(visible, "Got it, thank you.\n\nSam");
        assert!(quoted.expect("a quote").starts_with("On Mon,"));
    }

    #[test]
    fn plain_text_with_no_quoting_is_not_split() {
        let text = "Dinner on Thursday?\n\nSay by tomorrow and I will call them.\n";
        let (visible, quoted) = split_text(text);
        assert_eq!(visible, text.replace("\r\n", "\n"));
        assert_eq!(quoted, None);
    }

    #[test]
    fn a_quote_with_no_attribution_still_splits() {
        let text = "Sounds good.\n\n> Are you free Thursday?\n> Sam\n";
        let (visible, quoted) = split_text(text);
        assert_eq!(visible, "Sounds good.");
        assert!(quoted.expect("a quote").starts_with("> Are you free"));
    }

    #[test]
    fn a_message_that_is_only_quoted_lines_is_not_split() {
        let text = "> Are you free Thursday?\n> Sam\n";
        assert_eq!(split_text(text), (text.replace("\r\n", "\n"), None));
    }
}
