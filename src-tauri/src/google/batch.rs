// Gmail's batch endpoint, by hand.
//
// This is the highest leverage code in the Google client. Metadata hydration is the expensive part
// of a first sync at 20 units a message, and the difference between one HTTP round trip carrying
// fifty of them and fifty round trips is the difference between a first sync of minutes and one of
// an afternoon. There is no batch support in reqwest and no Rust Google client worth the
// dependency, so the multipart envelope is written and read here.
//
// The encoding and the decoding are pure functions over bytes with no client and no token in
// sight, because everything that can go wrong with a multipart envelope goes wrong in the bytes,
// and a test that needs a network is a test nobody runs.
//
// Two things the envelope makes easy to get wrong, both learned from Google's guide:
//
//   Each call inside a batch is metered separately. A batch of 50 `messages.get` is 1,000 units,
//   not 20, and the quota accountant in `api.rs` is told so.
//
//   Each call inside a batch fails separately. A 404 for a message deleted since it was listed and
//   a 429 for the account's minute arrive as the status of one part while the outer response is a
//   perfectly happy 200, so every part's status is read and none is assumed.

use serde::de::DeserializeOwned;

use crate::google::api::{self, ApiError};

/// Google allows 100 calls per batch and recommends no more than 50.
pub const MAX_PER_BATCH: usize = 50;

/// One call inside a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The `Content-ID`, which is how the answer is matched back to the question. Google is
    /// explicit that the order of the parts in the response is not to be relied on.
    pub id: String,
    pub method: &'static str,
    /// Absolute, query string included, as in `/gmail/v1/users/me/messages/abc?format=metadata`.
    pub path: String,
    pub body: Option<Vec<u8>>,
}

impl Part {
    pub fn get(id: impl Into<String>, path: impl Into<String>) -> Part {
        Part {
            id: id.into(),
            method: "GET",
            path: path.into(),
            body: None,
        }
    }

    pub fn post(id: impl Into<String>, path: impl Into<String>, body: Vec<u8>) -> Part {
        Part {
            id: id.into(),
            method: "POST",
            path: path.into(),
            body: Some(body),
        }
    }
}

/// One call's answer, with its own status. `body` is whatever came back, which for a failed part is
/// Google's ordinary error JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub id: String,
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: DeserializeOwned>(&self, context: &str, scope: &str) -> Result<T, ApiError> {
        let text = String::from_utf8_lossy(&self.body);
        if !self.is_success() {
            return Err(api::error_for(self.status, context, scope, None, &text));
        }
        serde_json::from_str(&text)
            .map_err(|e| ApiError::Other(format!("{context}: could not parse response: {e}")))
    }

    pub fn error(&self, context: &str, scope: &str) -> ApiError {
        api::error_for(
            self.status,
            context,
            scope,
            None,
            &String::from_utf8_lossy(&self.body),
        )
    }
}

/// A boundary that cannot occur in the payload it delimits. The random half is not decoration: a
/// forwarded message quoting an earlier multipart body really does contain other people's
/// boundaries.
pub fn boundary() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let suffix: u128 = rng.gen();
    format!("margin-mail-batch-{suffix:032x}")
}

/// The request envelope. `multipart/mixed`, one `application/http` part per call, each carrying its
/// own request line and its own `Content-ID`.
pub fn encode(boundary: &str, parts: &[Part]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(parts.len() * 512);
    for part in parts {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        out.extend_from_slice(b"Content-Type: application/http\r\n");
        out.extend_from_slice(format!("Content-ID: <{}>\r\n", part.id).as_bytes());
        out.extend_from_slice(b"Content-Transfer-Encoding: binary\r\n");
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(format!("{} {} HTTP/1.1\r\n", part.method, part.path).as_bytes());
        out.extend_from_slice(b"Accept: application/json\r\n");
        match &part.body {
            Some(body) => {
                out.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n");
                out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
                out.extend_from_slice(body);
                out.extend_from_slice(b"\r\n");
            }
            None => out.extend_from_slice(b"\r\n"),
        }
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

/// The boundary Google chose for its answer, which is never the one the request used.
pub fn boundary_from_content_type(value: &str) -> Option<String> {
    let at = value.to_ascii_lowercase().find("boundary=")?;
    let rest = value[at + "boundary=".len()..].trim();
    let rest = rest.split(';').next().unwrap_or(rest).trim();
    let rest = rest.trim_matches('"');
    (!rest.is_empty()).then(|| rest.to_string())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// The blank line between a header block and what follows it. Google sends CRLF; a proxy that has
/// rewritten it to LF should not cost a first sync.
fn split_headers(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let crlf = find(bytes, b"\r\n\r\n", 0);
    let lf = find(bytes, b"\n\n", 0);
    match (crlf, lf) {
        (Some(a), Some(b)) if b < a => Some((&bytes[..b], &bytes[b + 2..])),
        (Some(a), _) => Some((&bytes[..a], &bytes[a + 4..])),
        (None, Some(b)) => Some((&bytes[..b], &bytes[b + 2..])),
        (None, None) => None,
    }
}

fn header(block: &str, name: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

/// Google answers `Content-ID: <response-item1>` to a request that asked with `<item1>`.
fn strip_content_id(value: &str) -> String {
    let value = value.trim().trim_start_matches('<').trim_end_matches('>');
    value.strip_prefix("response-").unwrap_or(value).to_string()
}

fn parse_part(part: &[u8]) -> Result<Response, String> {
    let part = trim_leading_newlines(part);
    let (headers, rest) =
        split_headers(part).ok_or_else(|| "a batch part had no header block".to_string())?;
    let headers = String::from_utf8_lossy(headers);
    let id = header(&headers, "Content-ID")
        .map(|value| strip_content_id(&value))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            "a batch part had no Content-ID, so its answer cannot be matched to a call".to_string()
        })?;

    let (response_head, body) = split_headers(rest)
        .ok_or_else(|| format!("the batch part {id} carried no HTTP response"))?;
    let response_head = String::from_utf8_lossy(response_head);
    let status_line = response_head
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if !status_line.starts_with("HTTP/") {
        return Err(format!(
            "the batch part {id} began with {status_line:?} rather than an HTTP status line"
        ));
    }
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("the batch part {id} had an unreadable status line: {status_line:?}"))?;

    Ok(Response {
        id,
        status,
        body: trim_trailing_newlines(body).to_vec(),
    })
}

fn trim_leading_newlines(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && (bytes[start] == b'\r' || bytes[start] == b'\n') {
        start += 1;
    }
    &bytes[start..]
}

fn trim_trailing_newlines(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == b'\r' || bytes[end - 1] == b'\n') {
        end -= 1;
    }
    &bytes[..end]
}

/// The response envelope, back into one result per call.
///
/// A part that cannot be read is an error for the whole batch rather than a part quietly dropped:
/// silently losing forty-nine of fifty messages and calling the sync finished is the worst failure
/// this file could have, and it is exactly what skipping would produce.
pub fn decode(boundary: &str, body: &[u8]) -> Result<Vec<Response>, String> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut out = Vec::new();
    let mut cursor = match find(body, delimiter, 0) {
        Some(at) => at + delimiter.len(),
        None => {
            return Err(format!(
                "the batch response did not contain the boundary {boundary} Google announced"
            ))
        }
    };
    loop {
        if cursor >= body.len() {
            return Err("the batch response ended without a closing boundary".to_string());
        }
        if body[cursor..].starts_with(b"--") {
            return Ok(out);
        }
        let end = find(body, delimiter, cursor).ok_or_else(|| {
            "the batch response ended without a closing boundary".to_string()
        })?;
        out.push(parse_part(&body[cursor..end])?);
        cursor = end + delimiter.len();
    }
}

/// Sends one batch and returns one result per call, in the order the calls were given rather than
/// the order Google answered them.
pub async fn run(access_token: &str, parts: &[Part]) -> Result<Vec<Response>, ApiError> {
    if parts.is_empty() {
        return Ok(Vec::new());
    }
    if parts.len() > MAX_PER_BATCH {
        return Err(ApiError::Other(format!(
            "a batch of {} calls is over Google's recommended {MAX_PER_BATCH}",
            parts.len()
        )));
    }

    let boundary = boundary();
    let resp = api::HTTP
        .post(api::BATCH_URL)
        .bearer_auth(access_token)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/mixed; boundary={boundary}"),
        )
        .body(encode(&boundary, parts))
        .send()
        .await?;

    let status = resp.status().as_u16();
    let retry = api::retry_after(resp.headers());
    let announced = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(boundary_from_content_type);
    let body = resp.bytes().await?.to_vec();

    // The outer status is about the batch itself: a revoked token, a missing scope or the account's
    // whole minute. Per-call failures arrive inside a 200.
    if !(200..300).contains(&status) {
        return Err(api::error_for(
            status,
            "Gmail batch",
            api::SCOPE_MODIFY,
            retry,
            &String::from_utf8_lossy(&body),
        ));
    }

    let boundary = announced.ok_or_else(|| {
        ApiError::Other("Gmail's batch answer named no boundary in its Content-Type".to_string())
    })?;
    let mut answers = decode(&boundary, &body).map_err(ApiError::Other)?;
    answers.sort_by_key(|answer| {
        parts
            .iter()
            .position(|part| part.id == answer.id)
            .unwrap_or(usize::MAX)
    });
    Ok(answers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_parts(count: usize) -> Vec<Part> {
        (0..count)
            .map(|n| {
                let id = format!("18f9a2b3c4d5{n:04x}");
                Part::get(format!("m-{n}"), crate::google::api::metadata_path(&id))
            })
            .collect()
    }

    #[test]
    fn fifty_calls_encode_into_one_well_formed_multipart_body() {
        let parts = metadata_parts(50);
        let body = encode("test-boundary", &parts);
        let text = String::from_utf8(body).expect("the envelope is ASCII");

        // Fifty openers and one closer.
        assert_eq!(text.matches("--test-boundary").count(), 51);
        assert_eq!(text.matches("--test-boundary--").count(), 1);
        assert!(text.ends_with("--test-boundary--\r\n"));

        assert_eq!(text.matches("Content-Type: application/http").count(), 50);
        assert_eq!(text.matches("Content-ID: <m-").count(), 50);
        assert_eq!(text.matches(" HTTP/1.1\r\n").count(), 50);

        for (n, part) in parts.iter().enumerate() {
            assert!(text.contains(&format!("Content-ID: <m-{n}>\r\n")), "part {n}");
            assert!(
                text.contains(&format!("GET {} HTTP/1.1\r\n", part.path)),
                "part {n} lost its request line"
            );
        }
    }

    #[test]
    fn a_part_with_a_body_carries_its_own_content_length() {
        let body = br#"{"addLabelIds":["STARRED"]}"#.to_vec();
        let encoded = encode(
            "b",
            &[Part::post("m-0", "/gmail/v1/users/me/messages/abc/modify", body.clone())],
        );
        let text = String::from_utf8(encoded).expect("ascii");
        assert!(text.contains("POST /gmail/v1/users/me/messages/abc/modify HTTP/1.1\r\n"));
        assert!(text.contains(&format!("Content-Length: {}\r\n\r\n", body.len())));
        assert!(text.contains(r#"{"addLabelIds":["STARRED"]}"#));
    }

    /// Three metadata fetches out of one batch: one that worked, one for a message deleted between
    /// the list and the fetch, and one that ran the account's minute out. Google answers 200 to all
    /// of it, which is the whole reason every part's status is read.
    const CAPTURED: &str = concat!(
        "--batch_Fq0dSMj4uYA_AAyM2Ic4wLg\r\n",
        "Content-Type: application/http\r\n",
        "Content-ID: <response-m-0>\r\n",
        "\r\n",
        "HTTP/1.1 200 OK\r\n",
        "ETag: \"CLzYr8Xj4IkDEAAaBAgD\"\r\n",
        "Content-Type: application/json; charset=UTF-8\r\n",
        "Date: Thu, 03 Sep 2026 09:14:02 GMT\r\n",
        "Content-Length: 331\r\n",
        "\r\n",
        "{\"id\":\"18f9a2b3c4d50000\",\"threadId\":\"18f9a2b3c4d50000\",\"labelIds\":[\"UNREAD\",\"CATEGORY_PERSONAL\",\"INBOX\"],\"snippet\":\"The lease is attached, let me know\",\"sizeEstimate\":48213,\"historyId\":\"9912344\",\"internalDate\":\"1785808800000\",\"payload\":{\"headers\":[{\"name\":\"From\",\"value\":\"Ana Ruiz <ana@example.com>\"},{\"name\":\"Subject\",\"value\":\"The lease\"}]}}\r\n",
        "\r\n",
        "--batch_Fq0dSMj4uYA_AAyM2Ic4wLg\r\n",
        "Content-Type: application/http\r\n",
        "Content-ID: <response-m-1>\r\n",
        "\r\n",
        "HTTP/1.1 404 Not Found\r\n",
        "Content-Type: application/json; charset=UTF-8\r\n",
        "\r\n",
        "{\"error\":{\"code\":404,\"message\":\"Requested entity was not found.\",\"errors\":[{\"message\":\"Requested entity was not found.\",\"domain\":\"global\",\"reason\":\"notFound\"}],\"status\":\"NOT_FOUND\"}}\r\n",
        "\r\n",
        "--batch_Fq0dSMj4uYA_AAyM2Ic4wLg\r\n",
        "Content-Type: application/http\r\n",
        "Content-ID: <response-m-2>\r\n",
        "\r\n",
        "HTTP/1.1 429 Too Many Requests\r\n",
        "Content-Type: application/json; charset=UTF-8\r\n",
        "\r\n",
        "{\"error\":{\"code\":429,\"message\":\"User-rate limit exceeded.\",\"errors\":[{\"message\":\"User-rate limit exceeded.\",\"domain\":\"usageLimits\",\"reason\":\"rateLimitExceeded\"}],\"status\":\"RESOURCE_EXHAUSTED\"}}\r\n",
        "\r\n",
        "--batch_Fq0dSMj4uYA_AAyM2Ic4wLg--\r\n",
    );

    const CAPTURED_BOUNDARY: &str = "batch_Fq0dSMj4uYA_AAyM2Ic4wLg";

    #[test]
    fn a_captured_response_decodes_into_one_result_per_call() {
        let parts = decode(CAPTURED_BOUNDARY, CAPTURED.as_bytes()).expect("the capture decodes");
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["m-0", "m-1", "m-2"]
        );
        assert_eq!(
            parts.iter().map(|p| p.status).collect::<Vec<_>>(),
            [200, 404, 429]
        );

        let message: crate::google::api::Message = parts[0]
            .json("Gmail message fetch", crate::google::api::SCOPE_MODIFY)
            .expect("the successful part is a message");
        assert_eq!(message.id, "18f9a2b3c4d50000");
        assert_eq!(message.internal_date_ms(), 1_785_808_800_000);
        assert_eq!(message.header_pairs().len(), 2);
    }

    #[test]
    fn a_failed_part_keeps_its_own_meaning() {
        let parts = decode(CAPTURED_BOUNDARY, CAPTURED.as_bytes()).expect("the capture decodes");
        assert!(matches!(
            parts[1].error("Gmail message fetch", crate::google::api::SCOPE_MODIFY),
            ApiError::NotFound(_)
        ));
        assert!(matches!(
            parts[2].error("Gmail message fetch", crate::google::api::SCOPE_MODIFY),
            ApiError::RateLimited { .. }
        ));
    }

    #[test]
    fn a_part_with_no_status_line_fails_loudly_rather_than_being_skipped() {
        let mangled = CAPTURED.replace("HTTP/1.1 404 Not Found", "Not Found");
        let error = decode(CAPTURED_BOUNDARY, mangled.as_bytes())
            .expect_err("a part without a status line is not a part");
        assert!(error.contains("m-1"), "{error}");
        assert!(error.contains("status line"), "{error}");
    }

    #[test]
    fn a_part_with_no_content_id_fails_loudly_because_it_cannot_be_matched() {
        let mangled = CAPTURED.replace("Content-ID: <response-m-2>\r\n", "");
        let error = decode(CAPTURED_BOUNDARY, mangled.as_bytes())
            .expect_err("a part with no Content-ID cannot be matched to a call");
        assert!(error.contains("Content-ID"), "{error}");
    }

    #[test]
    fn a_truncated_response_fails_rather_than_returning_what_arrived() {
        let cut = &CAPTURED[..CAPTURED.len() / 2];
        let error = decode(CAPTURED_BOUNDARY, cut.as_bytes())
            .expect_err("half an envelope is not an envelope");
        assert!(error.contains("closing boundary"), "{error}");
    }

    #[test]
    fn the_answers_boundary_comes_out_of_its_content_type() {
        assert_eq!(
            boundary_from_content_type("multipart/mixed; boundary=batch_Fq0dSMj4uYA_AAyM2Ic4wLg"),
            Some(CAPTURED_BOUNDARY.to_string())
        );
        assert_eq!(
            boundary_from_content_type("multipart/mixed; boundary=\"quoted-one\"; charset=UTF-8"),
            Some("quoted-one".to_string())
        );
        assert_eq!(boundary_from_content_type("application/json"), None);
    }

    #[test]
    fn every_boundary_is_its_own() {
        assert_ne!(boundary(), boundary());
        assert!(boundary().starts_with("margin-mail-batch-"));
    }

    #[test]
    fn a_round_trip_survives_a_response_written_with_bare_line_feeds() {
        let lf = CAPTURED.replace("\r\n", "\n");
        let parts = decode(CAPTURED_BOUNDARY, lf.as_bytes()).expect("LF decodes too");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2].status, 429);
    }
}
