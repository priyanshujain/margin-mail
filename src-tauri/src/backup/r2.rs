// `impl BackupStore for R2`: the same three operations over S3 signature version four, for
// somebody who would rather their backup went to a bucket they own than to Drive.
//
// Cloudflare R2 is what this was written against and what the settings screen names, but nothing
// below is R2 specific beyond the region: it signs the way S3 does, so any endpoint that speaks
// path-style S3 with sigv4 works. The four fields are an endpoint, a bucket, an access key and a
// secret, and the secret never crosses the IPC boundary in the reading direction, which is why
// `dto::BackupSettings` carries the endpoint and the bucket and neither of the other two.
//
// The signing is by hand rather than through an SDK. `aws-sdk-s3` is a hundred crates and a
// runtime of its own for three verbs, and the signing is a page of code with two published test
// vectors to hold it to, which is what `tests.rs` does. Even the HMAC is here: the app does not
// depend on the `hmac` crate, and the construction over a hash it already has is six lines.

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::google::secrets;

use super::store::BackupStore;

/// The sealed credentials' name in the token store. Not an account id and unable to become one: a
/// Google `sub` is digits and an email address cannot carry a space.
const SECRET_ID: &str = "margin-mail r2 credentials";

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";

/// R2 has one region and calls it `auto`. It is not optional in a sigv4 signature even so, and
/// Cloudflare's own SDK signs against this string.
const REGION: &str = "auto";

/// Its own client rather than the Gmail layer's. They share no host, so one pool would never be
/// reused, and a backup upload's timeout has nothing to do with a metadata batch's.
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()
        .expect("could not build the HTTP client")
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub endpoint: String,
    pub bucket: String,
    pub access_key: String,
    pub secret: String,
}

impl Config {
    /// What the settings screen sends. Refused here rather than at the first upload, because a
    /// typed-in endpoint that is wrong should say so while the person still has the field open.
    pub fn from_fields(fields: &HashMap<String, String>) -> Result<Config, String> {
        let field = |camel: &str, snake: &str, label: &str| -> Result<String, String> {
            let value = fields
                .get(camel)
                .or_else(|| fields.get(snake))
                .map(|value| value.trim().to_string())
                .unwrap_or_default();
            if value.is_empty() {
                return Err(format!("The {label} is missing."));
            }
            Ok(value)
        };

        let endpoint = field("endpoint", "endpoint", "endpoint")?;
        let endpoint = endpoint.trim_end_matches('/').to_string();
        // Not https means the request that carries the signature, and everything the bucket policy
        // rests on, travels in the open. The blob itself is already encrypted; the credentials in
        // the header are not.
        if !endpoint.starts_with("https://") {
            return Err("The endpoint has to start with https://".to_string());
        }
        if url::Url::parse(&endpoint)
            .ok()
            .and_then(|url| url.host_str().map(|host| host.to_string()))
            .is_none()
        {
            return Err(format!("{endpoint} is not an endpoint address."));
        }

        let bucket = field("bucket", "bucket", "bucket name")?;
        if bucket.contains('/') {
            return Err("A bucket name is one word, with no slashes in it.".to_string());
        }

        Ok(Config {
            endpoint,
            bucket,
            access_key: field("accessKey", "access_key", "access key")?,
            secret: field("secret", "secret", "secret access key")?,
        })
    }
}

/// Sealed the way the OAuth tokens are, and for a stronger reason than the backup key: these are
/// credentials to somebody else's paid account, and unlike the key they cannot be derived again
/// from anything the person wrote down.
pub fn remember(config: &Config) -> Result<(), String> {
    let json = serde_json::to_string(config).map_err(|e| e.to_string())?;
    secrets::store(SECRET_ID, &json)
}

pub fn stored() -> Result<Option<Config>, String> {
    match secrets::load(SECRET_ID)? {
        Some(json) => serde_json::from_str(&json)
            .map(Some)
            .map_err(|e| format!("the stored R2 credentials are malformed: {e}")),
        None => Ok(None),
    }
}

pub fn forget() -> Result<(), String> {
    secrets::delete(SECRET_ID)
}

pub struct R2 {
    config: Config,
}

impl R2 {
    pub fn new(config: Config) -> R2 {
        R2 { config }
    }

    fn host(&self) -> Result<String, String> {
        let url = url::Url::parse(&self.config.endpoint).map_err(|e| e.to_string())?;
        let host = url
            .host_str()
            .ok_or_else(|| format!("{} is not an endpoint address.", self.config.endpoint))?;
        // The signed host has to be the host that goes on the wire, port and all, or the signature
        // covers a request nobody sent.
        Ok(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    }

    /// Builds and sends one signed request. Everything that differs between the three operations is
    /// a parameter, so there is one place that knows how a request is signed.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<reqwest::Response, String> {
        let host = self.host()?;
        let payload_sha = hex(&Sha256::digest(&body));
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

        let canonical_query = canonical_query(query);
        let headers = [
            ("host", host.clone()),
            ("x-amz-content-sha256", payload_sha.clone()),
            ("x-amz-date", amz_date.clone()),
        ];
        let authorization = authorization(
            &self.config.access_key,
            &self.config.secret,
            REGION,
            SERVICE,
            method.as_str(),
            path,
            &canonical_query,
            &headers,
            &payload_sha,
            &amz_date,
        );

        let mut url = format!("{}{path}", self.config.endpoint);
        if !canonical_query.is_empty() {
            url.push('?');
            url.push_str(&canonical_query);
        }

        HTTP.request(method, url)
            .header("x-amz-content-sha256", &payload_sha)
            .header("x-amz-date", &amz_date)
            .header(reqwest::header::AUTHORIZATION, authorization)
            .body(body)
            .send()
            .await
            .map_err(|e| format!("could not reach the bucket: {e}"))
    }

    fn path_for(&self, name: &str) -> String {
        format!(
            "/{}/{}",
            encode(&self.config.bucket, false),
            encode(name, true)
        )
    }
}

/// What the store said when it said no. The XML body is worth keeping: an S3 error names the code
/// in it, and "SignatureDoesNotMatch" is a different afternoon from "NoSuchBucket".
async fn refused(context: &str, resp: reqwest::Response) -> String {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    let detail = tag(&body, "Message").or_else(|| tag(&body, "Code"));
    match detail {
        Some(detail) => format!("{context} failed: {status}, {detail}"),
        None => format!("{context} failed: {status}"),
    }
}

impl BackupStore for R2 {
    fn put(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<(), String>> + Send {
        let path = self.path_for(name);
        let name = name.to_string();
        let bytes = bytes.to_vec();
        async move {
            let resp = self.send(reqwest::Method::PUT, &path, &[], bytes).await?;
            if !resp.status().is_success() {
                return Err(refused(&format!("Uploading {name}"), resp).await);
            }
            Ok(())
        }
    }

    fn get(&self, name: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send {
        let path = self.path_for(name);
        let name = name.to_string();
        async move {
            let resp = self
                .send(reqwest::Method::GET, &path, &[], Vec::new())
                .await?;
            if !resp.status().is_success() {
                return Err(refused(&format!("Downloading {name}"), resp).await);
            }
            resp.bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|e| format!("could not read {name}: {e}"))
        }
    }

    /// `list-type=2`, paged until the store says it has stopped truncating. The keys come back in
    /// an XML document; the two fields that matter are scraped out of it rather than parsed,
    /// because a dependency on an XML crate to read `<Key>` would be a poor trade.
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>, String>> + Send {
        let path = format!("/{}", encode(&self.config.bucket, false));
        let prefix = prefix.to_string();
        async move {
            let mut names = Vec::new();
            let mut token: Option<String> = None;
            loop {
                let mut query = vec![("list-type", "2"), ("prefix", prefix.as_str())];
                if let Some(token) = &token {
                    query.push(("continuation-token", token.as_str()));
                }
                let resp = self
                    .send(reqwest::Method::GET, &path, &query, Vec::new())
                    .await?;
                if !resp.status().is_success() {
                    return Err(refused("Listing the bucket", resp).await);
                }
                let body = resp
                    .text()
                    .await
                    .map_err(|e| format!("could not read the bucket listing: {e}"))?;
                names.extend(tags(&body, "Key"));
                match tag(&body, "NextContinuationToken") {
                    Some(next) if tag(&body, "IsTruncated").as_deref() == Some("true") => {
                        token = Some(next)
                    }
                    _ => return Ok(names),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Signature version four
// ---------------------------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        out.push_str(&format!("{byte:02x}"));
        out
    })
}

/// HMAC-SHA256, RFC 2104, over the hash the app already depends on. The block size is 64 bytes for
/// SHA-256 and every key sigv4 uses is shorter than that, but the long-key branch is here because
/// leaving it out would make this a function that is right for one caller rather than an HMAC.
pub(super) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; 64];
    let mut outer_pad = [0x5cu8; 64];
    for at in 0..64 {
        inner_pad[at] ^= block[at];
        outer_pad[at] ^= block[at];
    }
    let inner = Sha256::new()
        .chain_update(inner_pad)
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner)
        .finalize()
        .into()
}

/// The four nested HMACs that turn a secret into a key good for one day, one region and one
/// service. This is the derivation AWS publishes a test vector for, and `tests.rs` uses it.
pub(super) fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let start = format!("AWS4{secret}");
    let key = hmac_sha256(start.as_bytes(), date.as_bytes());
    let key = hmac_sha256(&key, region.as_bytes());
    let key = hmac_sha256(&key, service.as_bytes());
    hmac_sha256(&key, b"aws4_request")
}

/// Percent encoding as sigv4 defines it, which is stricter than a URL's: everything outside the
/// unreserved set is encoded, and the slash survives only where it is separating path segments.
pub(super) fn encode(value: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            b'/' if keep_slash => out.push('/'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Sorted by name, encoded, joined. Every query this module sends has distinct names, so there is
/// no tie to break on the value.
fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<String> = query
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name, false), encode(value, false)))
        .collect();
    pairs.sort();
    pairs.join("&")
}

/// The whole signature, as the `Authorization` header wants it.
///
/// Everything it needs is a parameter, including the moment and the header list, so that the
/// published test vectors can be run through this exact function rather than through a
/// reimplementation of it that might differ in the one place that matters.
#[allow(clippy::too_many_arguments)]
pub(super) fn authorization(
    access_key: &str,
    secret: &str,
    region: &str,
    service: &str,
    method: &str,
    uri: &str,
    canonical_query: &str,
    headers: &[(&str, String)],
    payload_sha: &str,
    amz_date: &str,
) -> String {
    let mut sorted: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_string()))
        .collect();
    sorted.sort();
    let canonical_headers: String = sorted
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers: Vec<&str> = sorted.iter().map(|(name, _)| name.as_str()).collect();
    let signed_headers = signed_headers.join(";");

    let canonical_request = format!(
        "{method}\n{uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_sha}"
    );
    let date = &amz_date[..8.min(amz_date.len())];
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex(&hmac_sha256(
        &signing_key(secret, date, region, service),
        to_sign.as_bytes(),
    ));
    format!(
        "{ALGORITHM} Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

// ---------------------------------------------------------------------------------------------
// Just enough XML
// ---------------------------------------------------------------------------------------------

pub(super) fn tags(xml: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        out.push(after[..end].to_string());
        rest = &after[end + close.len()..];
    }
    out
}

fn tag(xml: &str, name: &str) -> Option<String> {
    tags(xml, name).into_iter().next()
}
