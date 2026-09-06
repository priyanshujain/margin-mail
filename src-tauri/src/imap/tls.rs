// The socket under IMAP and SMTP, and the one place a certificate is decided about.
//
// Three things live here because all three are the same decision seen from different angles: how a
// connection is made, what happens when the certificate behind it does not verify, and what the
// user is asked when that happens somewhere it matters.
//
// The loopback is trusted without asking. Proton Bridge listens on 127.0.0.1 with a certificate it
// generated for itself, and so does every other local bridge; there is nothing between this
// process and that socket for anyone to impersonate, so a prompt there would be a dialog that
// teaches people to click through dialogs. Every other host is verified against the webpki roots,
// and a failure becomes a question with a fingerprint on it rather than a silent acceptance.
//
// One exception to that: a certificate that does not match the hostname is refused outright and
// never offered as a question. It is the single failure mode that looks exactly like an
// interception, and Thunderbird refuses it for the same reason.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, Error as TlsError, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::dto::{CertQuestion, Security};

/// How long to wait for a socket. Discovery probes twenty of these at once, so a host that black
/// holes has to give up before the person watching does.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------
// The stream
// ---------------------------------------------------------------------------------------------

/// A connection that may or may not be wrapped in TLS, so callers hold one type.
///
/// `async-imap` speaks futures-io and this speaks tokio-io, which is what `tokio_util::compat`
/// bridges at the call site. It is not bridged here because `mail-send` wants the tokio side.
pub enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Why a connection did not happen
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Refused {
    /// Nothing answered, or the name does not resolve, or it timed out.
    Unreachable(String),
    /// The certificate needs a decision before this host can be reached.
    Certificate(CertQuestion),
    /// The certificate names somebody else. Never a question, always a refusal.
    WrongHost { expected: String, found: String },
    /// The socket was fine and the credentials were not. Carries the server's own words, which for
    /// a mail server are usually worth showing: they are where "use an app password" comes from.
    Auth(String),
    Other(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Unreachable(m) => write!(f, "{m}"),
            Refused::Certificate(q) => {
                write!(f, "the certificate for {} is {}", q.host, q.reason)
            }
            Refused::WrongHost { expected, found } => write!(
                f,
                "the certificate is for {found}, not {expected}, so the connection was refused"
            ),
            Refused::Auth(m) => write!(f, "{m}"),
            Refused::Other(m) => write!(f, "{m}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The trust store
// ---------------------------------------------------------------------------------------------

/// Fingerprints somebody has accepted, keyed `host:port`. Held in memory and written through by
/// `imap::trust_cert`, which owns the file.
type Accepted = Mutex<std::collections::HashMap<String, String>>;

fn accepted() -> &'static Accepted {
    static ACCEPTED: OnceLock<Accepted> = OnceLock::new();
    ACCEPTED.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn remember(host: &str, port: u16, fingerprint: &str) {
    if let Ok(mut held) = accepted().lock() {
        held.insert(format!("{host}:{port}"), fingerprint.to_string());
    }
}

pub fn forget(host: &str, port: u16) {
    if let Ok(mut held) = accepted().lock() {
        held.remove(&format!("{host}:{port}"));
    }
}

pub fn all_accepted() -> Vec<(String, String)> {
    accepted()
        .lock()
        .map(|held| held.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn is_accepted(host: &str, port: u16, fingerprint: &str) -> bool {
    accepted()
        .lock()
        .map(|held| held.get(&format!("{host}:{port}")).map(String::as_str) == Some(fingerprint))
        .unwrap_or(false)
}

/// The loopback, where a self-signed certificate is the normal case rather than a warning sign.
pub fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// The certificate
// ---------------------------------------------------------------------------------------------

/// SHA-256 of the DER, uppercase hex in colon-separated pairs, which is how every other tool
/// prints one so a person can compare them without transcribing.
pub fn fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The parts of a certificate a person needs to decide about it.
pub fn describe(der: &[u8], host: &str, port: u16, reason: &str) -> CertQuestion {
    let (subject, issuer, expires_ms) = match x509_parser::parse_x509_certificate(der) {
        Ok((_, cert)) => (
            cert.subject().to_string(),
            cert.issuer().to_string(),
            cert.validity().not_after.timestamp() * 1_000,
        ),
        // A certificate that will not parse is still a certificate that was presented, and the
        // fingerprint is the part that matters for comparing it against what Bridge printed.
        Err(_) => (String::new(), String::new(), 0),
    };
    CertQuestion {
        host: host.to_string(),
        port,
        fingerprint: fingerprint(der),
        subject,
        issuer,
        expires_ms,
        reason: reason.to_string(),
    }
}

/// The verifier. Verifies properly first, and only then decides whether a failure is a question.
#[derive(Debug)]
struct Gatekeeper {
    inner: Arc<WebPkiServerVerifier>,
    host: String,
    port: u16,
    /// Filled in on the way past, so a refusal can be turned into a question with a real
    /// certificate in it. The handshake has already failed by the time anybody reads this.
    seen: Arc<Mutex<Option<Vec<u8>>>>,
}

impl ServerCertVerifier for Gatekeeper {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if let Ok(mut held) = self.seen.lock() {
            *held = Some(end_entity.as_ref().to_vec());
        }

        let verdict = self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now);
        let failure = match verdict {
            Ok(ok) => return Ok(ok),
            Err(failure) => failure,
        };

        // A name mismatch is the one failure that is indistinguishable from an interception, so it
        // is never offered as a choice. Everything else can be.
        if matches!(
            failure,
            TlsError::InvalidCertificate(CertificateError::NotValidForName)
        ) {
            return Err(failure);
        }

        // The loopback has no third party on it to be wrong about.
        if is_loopback(&self.host) {
            return Ok(ServerCertVerified::assertion());
        }

        if is_accepted(&self.host, self.port, &fingerprint(end_entity.as_ref())) {
            return Ok(ServerCertVerified::assertion());
        }

        Err(failure)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn roots() -> RootCertStore {
    let mut store = RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    store
}

/// A connector for one host, and the slot its certificate lands in.
///
/// The provider is named rather than taken from the process default on purpose: both crypto
/// providers are in this tree because of what reqwest pulls in, so there is no default to find and
/// building a config without one panics at runtime instead of failing to compile.
fn connector(host: &str, port: u16) -> Result<(TlsConnector, Arc<Mutex<Option<Vec<u8>>>>), Refused> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots()), provider.clone())
        .build()
        .map_err(|e| Refused::Other(e.to_string()))?;

    let seen = Arc::new(Mutex::new(None));
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Refused::Other(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Gatekeeper {
            inner,
            host: host.to_string(),
            port,
            seen: seen.clone(),
        }))
        .with_no_client_auth();
    // Nothing here speaks a protocol negotiated over ALPN, and offering one to a mail server that
    // does not expect it is a way to find out which servers mishandle it.
    config.alpn_protocols.clear();
    Ok((TlsConnector::from(Arc::new(config)), seen))
}

/// Turns a handshake failure into the answer the screen wants: a question when the certificate can
/// be decided about, a refusal when it cannot.
fn from_handshake(
    error: io::Error,
    seen: &Arc<Mutex<Option<Vec<u8>>>>,
    host: &str,
    port: u16,
) -> Refused {
    let text = error.to_string();
    let der = seen.lock().ok().and_then(|held| held.clone());

    let reason = if text.contains("NotValidForName") {
        let found = der
            .as_deref()
            .and_then(|der| x509_parser::parse_x509_certificate(der).ok())
            .map(|(_, cert)| cert.subject().to_string())
            .unwrap_or_else(|| "another host".to_string());
        return Refused::WrongHost {
            expected: host.to_string(),
            found,
        };
    } else if text.contains("Expired") {
        "expired"
    } else if text.contains("UnknownIssuer") || text.contains("NotValidForNameContext") {
        "unknown-issuer"
    } else if text.contains("certificate") || text.contains("Certificate") {
        "self-signed"
    } else {
        return Refused::Unreachable(text);
    };

    match der {
        Some(der) => Refused::Certificate(describe(&der, host, port, reason)),
        None => Refused::Other(text),
    }
}

// ---------------------------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------------------------

/// A plain TCP connection, with the timeout applied.
pub async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, Refused> {
    let target = format!("{host}:{port}");
    match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&target)).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(Refused::Unreachable(e.to_string())),
        Err(_) => Err(Refused::Unreachable(format!(
            "{target} did not answer within {} seconds",
            CONNECT_TIMEOUT.as_secs()
        ))),
    }
}

/// Wraps an open socket in TLS. This is also the STARTTLS upgrade: the caller has already sent the
/// command and read the answer, and hands the same socket over.
pub async fn upgrade(stream: TcpStream, host: &str, port: u16) -> Result<Stream, Refused> {
    let (connector, seen) = connector(host, port)?;
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| Refused::Other(format!("{host} is not a valid host name")))?;
    match connector.connect(name, stream).await {
        Ok(tls) => Ok(Stream::Tls(Box::new(tls))),
        Err(e) => Err(from_handshake(e, &seen, host, port)),
    }
}

/// A connection made the way a `Security` says to make it.
///
/// `StartTls` returns the socket still in the clear: the upgrade needs a protocol command that
/// only the caller knows how to send, so IMAP and SMTP each do their own and call `upgrade`.
pub async fn connect(host: &str, port: u16, security: Security) -> Result<Stream, Refused> {
    let stream = connect_tcp(host, port).await?;
    match security {
        Security::Tls => upgrade(stream, host, port).await,
        Security::StartTls | Security::Plain => Ok(Stream::Plain(stream)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loopback_is_recognised_by_name_and_by_either_address() {
        assert!(is_loopback("localhost"));
        assert!(is_loopback("LocalHost"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.0.1.1"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("imap.example.com"));
        // The one that matters: a host whose name merely contains the word.
        assert!(!is_loopback("localhost.example.com"));
        assert!(!is_loopback("10.0.0.1"));
    }

    #[test]
    fn a_fingerprint_is_the_form_a_person_can_compare_against_another_tool() {
        let printed = fingerprint(b"margin");
        assert_eq!(printed.len(), 32 * 3 - 1);
        assert!(printed.split(':').all(|pair| pair.len() == 2));
        assert!(printed
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':'));
        assert_eq!(printed, printed.to_uppercase());
    }

    #[test]
    fn an_accepted_fingerprint_is_remembered_per_host_and_port() {
        remember("mail.example.com", 993, "AA:BB");
        assert!(is_accepted("mail.example.com", 993, "AA:BB"));
        // A different port on the same host is a different decision, and so is a rotated key.
        assert!(!is_accepted("mail.example.com", 143, "AA:BB"));
        assert!(!is_accepted("mail.example.com", 993, "CC:DD"));
        forget("mail.example.com", 993);
        assert!(!is_accepted("mail.example.com", 993, "AA:BB"));
    }
}
