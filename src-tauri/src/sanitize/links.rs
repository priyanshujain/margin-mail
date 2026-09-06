// What a link in a message is allowed to be, and what gets taken out of it.
//
// Two separate jobs. The scheme rule is security: `javascript:` and `data:` are navigations into
// code and into a document the sender wrote, and neither belongs behind a click in a mail body.
// The parameter rule is privacy: the campaign identifiers below say nothing about where the link
// goes, only about who followed it, so a click is a report to the sender unless they come off.
//
// Removing a parameter can break a link. That is why the list is exact names rather than a prefix
// sweep, and why `link_cleaning` is a setting the reader can turn off.

use url::Url;

/// Schemes a link in a mail body may navigate to. `cid:` and `data:` are missing on purpose even
/// though the image side needs both: an image is fetched into a box, a link is a navigation.
const NAVIGABLE: &[&str] = &["http", "https", "mailto", "tel", "sms", "webcal", "geo"];

/// Identifiers whose only purpose is to say who clicked. Anything that also chooses what the page
/// shows stays, because a link that lands on the wrong page is worse than a link that reports.
const TRACKING_PARAMETERS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_name",
    "utm_reader",
    "utm_social",
    "utm_brand",
    "mc_cid",
    "mc_eid",
    "mkt_tok",
    "_hsenc",
    "_hsmi",
    "hsctatracking",
    "fbclid",
    "gclid",
    "gbraid",
    "wbraid",
    "dclid",
    "msclkid",
    "twclid",
    "ttclid",
    "igshid",
    "yclid",
    "li_fat_id",
    "epik",
    "vero_id",
    "vero_conv",
    "ck_subscriber_id",
    "oly_enc_id",
    "oly_anon_id",
    "ml_subscriber",
    "ml_subscriber_hash",
    "sc_campaign",
    "sc_channel",
    "sc_content",
    "sc_outcome",
    "trk_contact",
    "trk_msg",
    "trk_module",
    "trk_sid",
    "rb_clickid",
    "s_cid",
    "elqtrackid",
    "elqtrack",
    "spreport",
];

/// The value an `href` should end up with, or nothing when the link should not survive at all.
pub fn safe_href(value: &str, link_cleaning: bool) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    // An in-document jump is the one relative form that works inside a srcdoc, so it is the one
    // relative form that is kept.
    if trimmed.starts_with('#') {
        return Some(trimmed.to_string());
    }

    let parsed = Url::parse(trimmed).ok()?;
    if !NAVIGABLE.contains(&parsed.scheme()) {
        return None;
    }
    if !link_cleaning || !matches!(parsed.scheme(), "http" | "https") {
        return Some(trimmed.to_string());
    }
    Some(strip_tracking_parameters(trimmed))
}

/// The URL with the campaign identifiers removed, or the URL untouched when it had none. Untouched
/// matters: re-serialising a URL that needed nothing done to it changes percent encoding and case
/// for no reason, and every one of those differences ends up in a golden file.
pub fn strip_tracking_parameters(value: &str) -> String {
    let Ok(parsed) = Url::parse(value) else {
        return value.to_string();
    };
    if parsed.query().is_none() {
        return value.to_string();
    }
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(name, _)| !is_tracking_parameter(name))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    if kept.len() == parsed.query_pairs().count() {
        return value.to_string();
    }

    let mut cleaned = parsed.clone();
    if kept.is_empty() {
        cleaned.set_query(None);
    } else {
        cleaned.query_pairs_mut().clear().extend_pairs(kept);
    }
    cleaned.to_string()
}

fn is_tracking_parameter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    TRACKING_PARAMETERS.contains(&name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_and_documents_are_not_destinations() {
        assert_eq!(safe_href("javascript:alert(1)", true), None);
        assert_eq!(safe_href("  JavaScript:void(0)  ", true), None);
        assert_eq!(safe_href("data:text/html,<script>x</script>", true), None);
        assert_eq!(safe_href("vbscript:msgbox", true), None);
        assert_eq!(safe_href("file:///etc/passwd", true), None);
        assert_eq!(safe_href("cid:part@example.net", true), None);
    }

    #[test]
    fn ordinary_destinations_survive() {
        assert_eq!(
            safe_href("https://example.org/a", true).as_deref(),
            Some("https://example.org/a")
        );
        assert_eq!(
            safe_href("mailto:unsubscribe@example.org?subject=stop", true).as_deref(),
            Some("mailto:unsubscribe@example.org?subject=stop")
        );
        assert_eq!(safe_href("#section-2", true).as_deref(), Some("#section-2"));
    }

    #[test]
    fn a_relative_link_cannot_work_in_a_srcdoc_so_it_goes() {
        assert_eq!(safe_href("/stores", true), None);
        assert_eq!(safe_href("stores.html", true), None);
    }

    #[test]
    fn campaign_identifiers_come_off_and_the_rest_stays() {
        assert_eq!(
            strip_tracking_parameters("https://e.example/p?id=7&utm_source=n&mc_eid=abc&page=2"),
            "https://e.example/p?id=7&page=2"
        );
        assert_eq!(
            strip_tracking_parameters("https://e.example/p?utm_source=n"),
            "https://e.example/p"
        );
    }

    #[test]
    fn a_clean_url_is_returned_exactly_as_it_arrived() {
        let url = "https://thebrowser.example/unsubscribe?u=91827&id=8f3a1c";
        assert_eq!(strip_tracking_parameters(url), url);
        assert_eq!(safe_href(url, true).as_deref(), Some(url));
    }

    #[test]
    fn cleaning_off_leaves_the_url_alone() {
        let url = "https://e.example/p?utm_source=n";
        assert_eq!(safe_href(url, false).as_deref(), Some(url));
    }
}
