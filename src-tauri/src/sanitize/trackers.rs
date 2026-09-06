// Which remote images are there to watch you, and whose they are.
//
// A tracker is a remote image, so it is already blocked; naming it is what turns a silent block
// into a sentence the banner can say. Four tests, cheapest first: the host is on the list below, it
// is a pixel by its attributes, it is a pixel or invisible by its style, or its URL carries
// something shaped like the recipient's address.
//
// VENDORS ships with the app and is updated with the app. It is deliberately not fetched: a list
// downloaded at runtime would report every message opened to whoever serves the list, which is the
// thing this file exists to prevent. The entries are the hosts these products put in the `src` of
// their open-tracking pixel, taken from their own published sending documentation and from the raw
// source of mail they send. A host that is missing costs a name, not a leak, because the image was
// blocked before this function was asked about it.

use url::Url;

use super::Tag;

pub struct Vendor {
    pub name: &'static str,
    /// Matched as the whole host or as a suffix on a dot boundary.
    pub hosts: &'static [&'static str],
}

pub static VENDORS: &[Vendor] = &[
    Vendor { name: "Mailchimp", hosts: &["list-manage.com", "mailchimp.com", "mcusercontent.com", "campaign-archive.com"] },
    Vendor { name: "Mandrill", hosts: &["mandrillapp.com", "mandrill.com"] },
    Vendor { name: "HubSpot", hosts: &["hubspot.com", "hubspotemail.net", "hs-analytics.net", "hsforms.com", "hubspotlinks.com"] },
    Vendor { name: "SendGrid", hosts: &["sendgrid.net", "sendgrid.com", "sendgrid.info"] },
    Vendor { name: "Mailgun", hosts: &["mailgun.org", "mailgun.com", "mailgun.net"] },
    Vendor { name: "Amazon SES", hosts: &["awstrack.me"] },
    Vendor { name: "Postmark", hosts: &["postmarkapp.com", "pstmrk.it"] },
    Vendor { name: "SparkPost", hosts: &["sparkpostmail.com", "spmailtechno.com", "spmailtechnol.com"] },
    Vendor { name: "Constant Contact", hosts: &["rs6.net", "constantcontact.com", "ctctcdn.com", "ctctusercontent.com"] },
    Vendor { name: "Klaviyo", hosts: &["klaviyo.com", "klaviyomail.com", "kmail-lists.com"] },
    Vendor { name: "Braze", hosts: &["braze.com", "braze.eu", "appboy.com", "appboycdn.com"] },
    Vendor { name: "Customer.io", hosts: &["customer.io", "customeriomail.com"] },
    Vendor { name: "Iterable", hosts: &["iterable.com", "links.iterable.com"] },
    Vendor { name: "Marketo", hosts: &["marketo.com", "marketo.net", "mktoresp.com", "mktdns.com"] },
    Vendor { name: "Pardot", hosts: &["pardot.com"] },
    Vendor { name: "Salesforce Marketing Cloud", hosts: &["exacttarget.com", "exct.net", "et.exacttarget.com"] },
    Vendor { name: "Oracle Responsys", hosts: &["responsys.net", "rsys.net"] },
    Vendor { name: "Oracle Eloqua", hosts: &["eloqua.com", "en25.com"] },
    Vendor { name: "Sailthru", hosts: &["sailthru.com", "sail-track.com", "sail-horizon.com"] },
    Vendor { name: "ActiveCampaign", hosts: &["activehosted.com", "activecampaign.com"] },
    Vendor { name: "ConvertKit", hosts: &["convertkit-mail.com", "convertkit-mail2.com", "convertkit.com"] },
    Vendor { name: "Drip", hosts: &["getdrip.com", "dripemail2.com"] },
    Vendor { name: "Omnisend", hosts: &["omnisend.com", "omnisrc.com"] },
    Vendor { name: "Beehiiv", hosts: &["beehiiv.com", "beehiiv.net"] },
    Vendor { name: "Intercom", hosts: &["intercom-mail.com", "intercomcdn.com", "intercom.io"] },
    Vendor { name: "Litmus", hosts: &["emltrk.com", "litmus.com"] },
    Vendor { name: "Mailtrack", hosts: &["mailtrack.io"] },
    Vendor { name: "Streak", hosts: &["mailfoogae.appspot.com", "streak.com"] },
    Vendor { name: "Yesware", hosts: &["yesware.com"] },
    Vendor { name: "Mixmax", hosts: &["mixmax.com"] },
    Vendor { name: "Bananatag", hosts: &["bananatag.com", "bl-1.com"] },
    Vendor { name: "Outreach", hosts: &["outreach.io"] },
    Vendor { name: "Salesloft", hosts: &["salesloft.com", "salesloft.io"] },
    Vendor { name: "Zoho Campaigns", hosts: &["zohocampaigns.com", "zcsend.net", "campaigns.zoho.com"] },
    Vendor { name: "MailerLite", hosts: &["mailerlite.com", "ml-attach.com", "mlsend.com"] },
    Vendor { name: "Emarsys", hosts: &["emarsys.net", "emarsys.com"] },
    Vendor { name: "Acoustic", hosts: &["pages05.net", "silverpop.com", "mkt51.net"] },
    Vendor { name: "Adobe Campaign", hosts: &["neolane.net", "adobecampaign.com"] },
];

/// The vendor for a host, or nothing when it is not one we ship a name for.
pub fn vendor_for_host(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    VENDORS.iter().find_map(|vendor| {
        vendor
            .hosts
            .iter()
            .any(|listed| host == *listed || host.ends_with(&format!(".{listed}")))
            .then_some(vendor.name)
    })
}

/// What the banner calls this image, or nothing when it is an ordinary remote image.
///
/// The host is the fallback name on purpose: "one tracking pixel from meridianproperties.in" is a
/// true and useful sentence even when the sender rolled their own beacon.
pub fn classify(url: &str, tag: &Tag) -> Option<String> {
    let host = Url::parse(url).ok()?.host_str()?.to_string();

    if let Some(vendor) = vendor_for_host(&host) {
        return Some(vendor.to_string());
    }
    if is_pixel_by_attributes(tag) || is_hidden_by_style(tag) || carries_recipient_token(url) {
        return Some(host);
    }
    None
}

fn is_pixel_by_attributes(tag: &Tag) -> bool {
    let tiny = |name: &str| {
        tag.attr(name)
            .map(|value| {
                let value = value.trim().trim_end_matches("px");
                matches!(value.parse::<f32>(), Ok(number) if number <= 1.0)
            })
            .unwrap_or(false)
    };
    tiny("width") || tiny("height")
}

fn is_hidden_by_style(tag: &Tag) -> bool {
    let Some(style) = tag.attr("style") else {
        return false;
    };
    let style: String = style
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();

    if style.contains("display:none") || style.contains("visibility:hidden") {
        return true;
    }
    if style.contains("opacity:0") && !style.contains("opacity:0.") {
        return true;
    }
    ["width:", "height:", "max-height:", "max-width:"]
        .iter()
        .any(|property| tiny_length(&style, property))
}

fn tiny_length(style: &str, property: &str) -> bool {
    let Some(at) = style.find(property) else {
        return false;
    };
    let rest = &style[at + property.len()..];
    let value: String = rest
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    matches!(value.parse::<f32>(), Ok(number) if number <= 1.0)
}

/// A URL that carries the recipient's own address, in the clear or base64'd, is not fetching an
/// image on the recipient's behalf.
fn carries_recipient_token(url: &str) -> bool {
    let decoded = percent_decode(url);
    if contains_address(&decoded) {
        return true;
    }
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    parsed.query_pairs().any(|(_, value)| {
        value.len() >= 12
            && base64_decode(value.as_ref())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|text| contains_address(&text))
                .unwrap_or(false)
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value))
        .ok()
}

fn contains_address(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        if *byte != b'@' || at == 0 {
            continue;
        }
        let local_ok = bytes[..at]
            .iter()
            .rev()
            .take_while(|byte| is_address_byte(**byte))
            .count()
            > 0;
        let domain: Vec<u8> = bytes[at + 1..]
            .iter()
            .copied()
            .take_while(|byte| is_address_byte(*byte))
            .collect();
        let domain_ok = domain.contains(&b'.')
            && domain
                .rsplit(|byte| *byte == b'.')
                .next()
                .map(|tld| tld.len() >= 2 && tld.iter().all(|byte| byte.is_ascii_alphabetic()))
                .unwrap_or(false);
        if local_ok && domain_ok {
            return true;
        }
    }
    false
}

fn is_address_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sanitize::scan;

    fn img(markup: &str) -> Tag {
        scan(markup).into_iter().next().expect("a tag")
    }

    #[test]
    fn a_listed_host_is_named_by_its_vendor() {
        assert_eq!(vendor_for_host("track.hubspot.com"), Some("HubSpot"));
        assert_eq!(vendor_for_host("HUBSPOT.COM"), Some("HubSpot"));
        assert_eq!(vendor_for_host("nothubspot.com"), None);
        assert_eq!(vendor_for_host("example.org"), None);
    }

    #[test]
    fn a_one_pixel_image_is_a_tracker_under_its_own_host() {
        let tag = img(r#"<img src="https://shop.example/o/open.png?id=1" width="1" height="1">"#);
        assert_eq!(
            classify("https://shop.example/o/open.png?id=1", &tag).as_deref(),
            Some("shop.example")
        );
    }

    #[test]
    fn a_real_image_is_not_a_tracker() {
        let tag = img(r#"<img src="https://shop.example/photo.jpg" width="640" height="420">"#);
        assert_eq!(classify("https://shop.example/photo.jpg", &tag), None);
    }

    #[test]
    fn style_can_hide_a_pixel_the_attributes_do_not() {
        let tag = img(r#"<img src="https://shop.example/p.gif" style="width:1px;height:1px">"#);
        assert!(classify("https://shop.example/p.gif", &tag).is_some());
        let hidden = img(r#"<img src="https://shop.example/p.gif" style="display: none">"#);
        assert!(classify("https://shop.example/p.gif", &hidden).is_some());
    }

    #[test]
    fn a_recipient_address_in_the_url_gives_it_away() {
        let tag = img(r#"<img src="https://shop.example/beacon.png?r=pj%4073ai.org" width="600">"#);
        assert!(classify("https://shop.example/beacon.png?r=pj%4073ai.org", &tag).is_some());
    }

    #[test]
    fn a_base64_recipient_token_gives_it_away_too() {
        let encoded = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode("pj@73ai.org")
        };
        let url = format!("https://shop.example/x.png?u={encoded}");
        let tag = img(&format!(r#"<img src="{url}" width="600">"#));
        assert!(classify(&url, &tag).is_some());
    }

    #[test]
    fn an_ordinary_query_string_is_not_a_token() {
        let url = "https://shop.example/hero.png?w=640&fit=crop";
        let tag = img(&format!(r#"<img src="{url}" width="640" height="420">"#));
        assert_eq!(classify(url, &tag), None);
    }
}
