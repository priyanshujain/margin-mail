// The `.eml` corpus, compiled into the test binary.
//
// Every file in `fixtures/` is a real message as it comes off the wire: CRLF throughout, headers
// folded the way the sending client folded them, bodies in whatever encoding the sender used. They
// are read as bytes rather than as `&str` because half of them are not UTF-8, which is the point:
// ISO-8859-1 and ISO-2022-JP still arrive daily and a parser that has only seen UTF-8 has not been
// tested. Nothing here is generated at test time, so a fixture cannot quietly change shape between
// the parser's tests and the engine's.

macro_rules! corpus {
    ($($name:ident => $file:literal,)+) => {
        $(pub const $name: &[u8] = include_bytes!(concat!("../fixtures/", $file));)+

        /// The whole corpus, for a test that has to hold every message to the same rule.
        pub fn all() -> Vec<(&'static str, &'static [u8])> {
            vec![$(($file, $name),)+]
        }
    };
}

corpus! {
    ATTACHMENT_FILENAME_ENCODED_WORD => "attachment-filename-encoded-word.eml",
    CALENDAR_INVITE => "calendar-invite.eml",
    CHARSET_ISO_2022_JP => "charset-iso-2022-jp.eml",
    CHARSET_ISO_8859_1 => "charset-iso-8859-1.eml",
    CID_MISSING_PART => "cid-missing-part.eml",
    DISPLAY_NAME_COMMA_QUOTES => "display-name-comma-quotes.eml",
    ENCODED_WORDS_SPLIT_UTF8 => "encoded-words-split-utf8.eml",
    ENCODED_WORDS_SUBJECT_B => "encoded-words-subject-b.eml",
    ENCODED_WORDS_SUBJECT_Q => "encoded-words-subject-q.eml",
    GROUP_ADDRESS_LIST => "group-address-list.eml",
    HEADER_ODDITIES => "header-oddities.eml",
    HTML_HOSTILE => "html-hostile.eml",
    INLINE_IMAGE_ATTACHMENT_DISPOSITION => "inline-image-attachment-disposition.eml",
    INLINE_IMAGE_CID => "inline-image-cid.eml",
    INVITE_DAYLIGHT_SAVING => "invite-daylight-saving.eml",
    NESTED_MULTIPART => "nested-multipart.eml",
    NEWSLETTER_LIST_UNSUBSCRIBE => "newsletter-list-unsubscribe.eml",
    NO_MESSAGE_ID => "no-message-id.eml",
    OUTLOOK_REPLY => "outlook-reply.eml",
    PLAIN_TEXT => "plain-text.eml",
    QUOTED_PRINTABLE_SOFT_BREAKS => "quoted-printable-soft-breaks.eml",
    QUOTED_REPLY_BLOCKQUOTE => "quoted-reply-blockquote.eml",
    QUOTED_REPLY_PLAIN => "quoted-reply-plain.eml",
    RECEIPT_NO_REPLY => "receipt-no-reply.eml",
    REPLY_REFERENCES_FOLDED => "reply-references-folded.eml",
    RFC2231_FILENAME => "rfc2231-filename.eml",
    SCREENER_FIRST_CONTACT => "screener-first-contact.eml",
    SERVICE_ON_BEHALF => "service-on-behalf.eml",
    TRACKING_PIXEL => "tracking-pixel.eml",
    UTF8_RAW_HEADERS => "utf8-raw-headers.eml",
}

/// The one fixture with no `Message-ID`, kept named here because the thread key rule falls all the
/// way through on it and more than one test wants that case.
pub const WITHOUT_MESSAGE_ID: &[u8] = NO_MESSAGE_ID;

mod tests {
    use super::all;
    use crate::provider::fake::FakeMessage;
    use mail_parser::MessageParser;

    /// A date outside this range is a parser that gave up and returned zero rather than a message
    /// from an unusual year.
    const EARLIEST_MS: i64 = 946_684_800_000; // 2000-01-01
    const LATEST_MS: i64 = 4_102_444_800_000; // 2100-01-01

    #[test]
    fn every_fixture_is_a_message() {
        for (name, raw) in all() {
            assert!(
                raw.windows(4).any(|window| window == b"\r\n\r\n"),
                "{name} has no blank line between the headers and the body"
            );
            assert!(
                !raw.split(|byte| *byte == b'\n')
                    .any(|line| !line.is_empty() && !line.ends_with(b"\r")),
                "{name} has a bare LF, so it is not what the wire delivers"
            );

            let message = FakeMessage::from_eml("id-1", "thread-1", &["INBOX"], raw);
            assert_eq!(message.raw, raw, "{name} did not survive from_eml intact");
            assert!(
                message.date_ms > EARLIEST_MS && message.date_ms < LATEST_MS,
                "{name} came out with an implausible date: {}",
                message.date_ms
            );

            let parsed = MessageParser::default()
                .parse(&message.raw)
                .unwrap_or_else(|| panic!("{name} did not parse at all"));
            let from = parsed
                .from()
                .and_then(|address| address.first())
                .and_then(|address| address.address())
                .unwrap_or_default();
            assert!(
                from.contains('@'),
                "{name} came out with no From address, got {from:?}"
            );
        }
    }
}
