// One authenticated IMAP connection.
//
// OWNED BY THE IMAP PROVIDER PACKAGE.
//
// Everything here happens once per connection and nothing here knows what a message is. The order
// is the order the protocol insists on: greeting, capabilities, STARTTLS if the security says so,
// capabilities again because they change across the upgrade, authenticate, capabilities a third
// time because they change again, then `ID`.
//
// `async-imap` is compiled against tokio here rather than async-std, so `tls::Stream` goes in
// directly and there is no `tokio_util::compat` in the middle. The only thing the stream is
// missing is a `Debug`, which `Client<T>` requires and `tls.rs` is frozen without, so `Socket` is
// that one impl and nothing else.
//
// The mechanism preference is Thunderbird's: CRAM-MD5, then SASL PLAIN, then SASL LOGIN, then the
// bare LOGIN command unless the server has disabled it. MD5 lives at the bottom of this file
// because CRAM-MD5 needs one and nothing else in this tree does; the RFC's own vectors are under
// it, which is the only reason it is defensible to have written it out by hand.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_imap::error::Error as ImapError;
use async_imap::imap_proto::{Response, ResponseCode, Status};
use async_imap::types::Capabilities;
use async_imap::{Authenticator, Client};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

use super::tls::{self, Refused};
use crate::dto::{Security, ServerConfig};

/// What this client tells a server it is, in the RFC 2971 `ID` exchange.
const CLIENT_NAME: &str = "Margin Mail";

/// The tag on the one command sent by hand rather than through `async-imap`. It has to be a shape
/// the crate's own generator will never produce, so a stray answer cannot be mistaken for ours.
const OUR_TAG: &str = "mmcap";

// ---------------------------------------------------------------------------------------------
// The socket
// ---------------------------------------------------------------------------------------------

/// `tls::Stream` with a `Debug`, which is the whole of it.
///
/// `async_imap::Client<T>` is bounded on `T: Debug` so its own `#[derive(Debug)]` compiles, and it
/// never prints one. The stream itself is frozen in `tls.rs`, so the impl belongs here rather than
/// there, and it deliberately says nothing: a socket's address is not something to leak into a log.
pub struct Socket(tls::Stream);

impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Socket")
    }
}

impl AsyncRead for Socket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Socket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

/// One live connection, logged in, with its capabilities read.
pub struct Session {
    pub capabilities: Vec<String>,
    inner: async_imap::Session<Socket>,
}

impl Session {
    /// Case-insensitively, because servers disagree about the case of their own capability names
    /// and `LOGINDISABLED` matters far too much to miss over one.
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|held| held.eq_ignore_ascii_case(capability))
    }

    pub fn imap(&mut self) -> &mut async_imap::Session<Socket> {
        &mut self.inner
    }

    /// Runs one command, handing every untagged response it produces to `line` on the way past.
    ///
    /// Every typed helper in `async-imap` that returns more than one response returns it as a
    /// `futures::Stream`, and `futures` is not a dependency of this crate. Reading the responses
    /// here instead costs one loop and buys back the two things those helpers throw away: the
    /// untagged VANISHED lines, which are the whole of QRESYNC's account of what was deleted, and
    /// the certainty that nothing was dropped, because the crate's overflow channel is bounded at
    /// a hundred and discards in silence once it is full.
    ///
    /// A callback rather than a list because the crate's `ResponseData` is crate private: it can
    /// be held on the stack but it cannot be named, so a caller takes what it wants from the
    /// borrowed `Response` and keeps that.
    pub async fn command<F>(&mut self, command: &str, mut line: F) -> Result<(), ImapError>
    where
        F: FnMut(&Response<'_>),
    {
        let id = self.inner.run_command(command).await?;
        loop {
            let Some(response) = self.inner.read_response().await? else {
                return Err(ImapError::ConnectionLost);
            };
            let parsed = response.parsed();
            if let Response::Done {
                tag,
                status,
                code,
                information,
            } = parsed
            {
                if tag == &id {
                    return outcome(status, code.as_ref(), information.as_deref());
                }
            }
            line(parsed);
        }
    }

    /// A command whose only interesting answer is whether it worked.
    pub async fn run(&mut self, command: &str) -> Result<(), ImapError> {
        self.command(command, |_| {}).await
    }

    /// `APPEND`, which is the one command whose argument does not fit on the command line.
    ///
    /// The literal handshake is written out because the crate's own `append` is fine but its
    /// siblings are not, and one command going through a different path from the rest is how a
    /// buffer ends up half read.
    pub async fn append(
        &mut self,
        mailbox: &str,
        flags: &str,
        raw: &[u8],
    ) -> Result<(), ImapError> {
        let id = self
            .inner
            .run_command(&format!(
                "APPEND {} ({}) {{{}}}",
                quoted(mailbox),
                flags,
                raw.len()
            ))
            .await?;

        loop {
            let Some(response) = self.inner.read_response().await? else {
                return Err(ImapError::ConnectionLost);
            };
            let ready = match response.parsed() {
                Response::Continue { .. } => Some(Ok(())),
                Response::Done {
                    tag,
                    status,
                    code,
                    information,
                } if tag == &id => Some(
                    outcome(status, code.as_ref(), information.as_deref())
                        .and(Err(ImapError::Append)),
                ),
                _ => None,
            };
            match ready {
                Some(Ok(())) => break,
                Some(Err(error)) => return Err(error),
                None => {}
            }
        }

        let socket = self.inner.get_mut();
        socket.write_all(raw).await?;
        socket.write_all(b"\r\n").await?;
        socket.flush().await?;

        loop {
            let Some(response) = self.inner.read_response().await? else {
                return Err(ImapError::ConnectionLost);
            };
            let finished = match response.parsed() {
                Response::Done {
                    tag,
                    status,
                    code,
                    information,
                } if tag == &id => Some(outcome(status, code.as_ref(), information.as_deref())),
                _ => None,
            };
            if let Some(finished) = finished {
                return finished;
            }
        }
    }

    /// A polite goodbye. A server that will not hear it is a server we are done with anyway.
    pub async fn close(mut self) {
        let _ = self.inner.logout().await;
    }
}

/// A tagged answer as a `Result`, with the server's own sentence kept rather than a debug format
/// of the response code, because that sentence is what the connect screen shows somebody.
fn outcome(
    status: &Status,
    code: Option<&ResponseCode<'_>>,
    information: Option<&str>,
) -> Result<(), ImapError> {
    if matches!(status, Status::Ok) {
        return Ok(());
    }
    let said = match information {
        Some(said) => said.to_string(),
        None => format!("{code:?}"),
    };
    match status {
        Status::No => Err(ImapError::No(said)),
        _ => Err(ImapError::Bad(said)),
    }
}

/// An IMAP quoted string. Mailbox names come off the wire in modified UTF-7 and go back exactly as
/// they came, so this only has to escape the two characters the grammar reserves.
pub fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Connects, upgrades if the security says to, authenticates, and drops the connection. This is
/// what the connect screen's test button runs.
pub async fn check(server: &ServerConfig, password: &str) -> Result<(), Refused> {
    open(server, password).await?.close().await;
    Ok(())
}

/// Connects and keeps the connection.
pub async fn open(server: &ServerConfig, password: &str) -> Result<Session, Refused> {
    let host = server.host.as_str();
    let port = server.port;

    let mut client = Client::new(Socket(tls::connect(host, port, server.security).await?));

    // The greeting is not optional and it is not always a welcome: a server that has decided it
    // does not want to talk to this address says so here and closes.
    let greeting = client
        .read_response()
        .await
        .map_err(|e| Refused::Unreachable(e.to_string()))?
        .ok_or_else(|| Refused::Unreachable(format!("{host} closed the connection")))?;
    let mut capabilities = match greeting.parsed() {
        Response::Data { status, code, information } => {
            match status {
                Status::Bye => {
                    return Err(Refused::Unreachable(
                        information.as_deref().unwrap_or("the server said goodbye").to_string(),
                    ))
                }
                // Nothing here can use a connection that is authenticated already, because
                // `async-imap` only makes a session out of a login it performed itself.
                Status::PreAuth => {
                    return Err(Refused::Other(
                        "this server authenticates by itself, which this app cannot use"
                            .to_string(),
                    ))
                }
                _ => {}
            }
            match code {
                Some(async_imap::imap_proto::ResponseCode::Capabilities(listed)) => {
                    listed.iter().map(borrowed_capability).collect()
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };

    if matches!(server.security, Security::StartTls) {
        if capabilities.is_empty() {
            capabilities = ask_capabilities(&mut client).await?;
        }
        if !capabilities.iter().any(|held| held == "STARTTLS") {
            return Err(Refused::Other(format!(
                "{host} does not offer STARTTLS on port {port}"
            )));
        }
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .map_err(|e| Refused::Other(format!("{host} refused to start TLS: {e}")))?;

        // Safe to take the socket back here and nowhere else: a server must send nothing between
        // the OK and the TLS handshake, so there is nothing left in the read buffer to lose.
        let tls::Stream::Plain(plain) = client.into_inner().0 else {
            return Err(Refused::Other("that connection was already encrypted".to_string()));
        };
        client = Client::new(Socket(tls::upgrade(plain, host, port).await?));
        // Everything said before the upgrade was said in the clear and is worth nothing now.
        capabilities.clear();
    }

    if capabilities.is_empty() {
        capabilities = ask_capabilities(&mut client).await?;
    }

    let mechanism = mechanism(&capabilities).ok_or_else(|| {
        Refused::Auth(format!(
            "{host} offers no way to log in with a password that this app knows"
        ))
    })?;
    let mut inner = authenticate(client, mechanism, &server.username, password).await?;

    // Read again: a server may advertise a different set once it knows who is asking, and
    // CONDSTORE and QRESYNC are commonly among them.
    let capabilities = match inner.capabilities().await {
        Ok(listed) => owned_capabilities(&listed),
        Err(e) => return Err(Refused::Other(e.to_string())),
    };

    let mut session = Session { capabilities, inner };

    // NetEase (163.com, 126.com, yeah.net) answers SELECT with "Unsafe Login" until a client has
    // identified itself, and every other server treats this as a courtesy. One round trip.
    if session.has("ID") {
        let _ = session
            .imap()
            .id([
                ("name", Some(CLIENT_NAME)),
                ("version", Some(env!("CARGO_PKG_VERSION"))),
            ])
            .await;
    }
    Ok(session)
}

/// Asks for the capability list and reads the untagged answer.
///
/// Written by hand because `Client` has no public way to do it: `run_command` is crate private and
/// `run_command_and_check_ok` throws away everything it reads on the way to the tagged line. The
/// command goes straight at the socket, which is exactly what the crate's own encoder does with
/// it, and the answers come back through the crate's parser.
async fn ask_capabilities(client: &mut Client<Socket>) -> Result<Vec<String>, Refused> {
    let socket = client.get_mut();
    socket
        .write_all(format!("{OUR_TAG} CAPABILITY\r\n").as_bytes())
        .await
        .map_err(|e| Refused::Unreachable(e.to_string()))?;
    socket
        .flush()
        .await
        .map_err(|e| Refused::Unreachable(e.to_string()))?;

    let mut found: Vec<String> = Vec::new();
    loop {
        let response = client
            .read_response()
            .await
            .map_err(|e| Refused::Unreachable(e.to_string()))?
            .ok_or_else(|| Refused::Unreachable("the connection closed".to_string()))?;
        match response.parsed() {
            Response::Capabilities(listed) => {
                found = listed.iter().map(borrowed_capability).collect();
            }
            Response::Done { tag, status, information, .. } if tag.0 == OUR_TAG => {
                if !matches!(status, Status::Ok) {
                    return Err(Refused::Other(
                        information
                            .as_deref()
                            .unwrap_or("the server would not say what it can do")
                            .to_string(),
                    ));
                }
                return Ok(found);
            }
            _ => {}
        }
    }
}

fn borrowed_capability(capability: &async_imap::imap_proto::Capability<'_>) -> String {
    use async_imap::imap_proto::Capability;
    match capability {
        Capability::Imap4rev1 => "IMAP4REV1".to_string(),
        Capability::Auth(name) => format!("AUTH={}", name.to_uppercase()),
        Capability::Atom(name) => name.to_uppercase(),
    }
}

fn owned_capabilities(listed: &Capabilities) -> Vec<String> {
    use async_imap::types::Capability;
    let mut out: Vec<String> = listed
        .iter()
        .map(|capability| match capability {
            Capability::Imap4rev1 => "IMAP4REV1".to_string(),
            Capability::Auth(name) => format!("AUTH={}", name.to_uppercase()),
            Capability::Atom(name) => name.to_uppercase(),
        })
        .collect();
    // The set arrives from a hash set, and an order that changes between runs is an order that
    // makes a failure impossible to reproduce.
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mechanism {
    CramMd5,
    Plain,
    Login,
    /// The bare `LOGIN user pass` command, which is not SASL and is the last resort.
    Command,
}

/// Thunderbird's order. CRAM-MD5 first because it is the only one of these that does not put the
/// password on the wire, then the two SASL mechanisms, then the command, which the server is
/// allowed to forbid outright and which is never tried when it has said so.
fn mechanism(capabilities: &[String]) -> Option<Mechanism> {
    let has = |name: &str| {
        capabilities
            .iter()
            .any(|held| held.eq_ignore_ascii_case(name))
    };
    if has("AUTH=CRAM-MD5") {
        return Some(Mechanism::CramMd5);
    }
    if has("AUTH=PLAIN") {
        return Some(Mechanism::Plain);
    }
    if has("AUTH=LOGIN") {
        return Some(Mechanism::Login);
    }
    (!has("LOGINDISABLED")).then_some(Mechanism::Command)
}

async fn authenticate(
    client: Client<Socket>,
    mechanism: Mechanism,
    username: &str,
    password: &str,
) -> Result<async_imap::Session<Socket>, Refused> {
    let outcome = match mechanism {
        Mechanism::CramMd5 => {
            client
                .authenticate(
                    "CRAM-MD5",
                    CramMd5 {
                        username: username.to_string(),
                        password: password.to_string(),
                    },
                )
                .await
        }
        Mechanism::Plain => {
            client
                .authenticate(
                    "PLAIN",
                    PlainSasl {
                        username: username.to_string(),
                        password: password.to_string(),
                    },
                )
                .await
        }
        Mechanism::Login => {
            client
                .authenticate(
                    "LOGIN",
                    LoginSasl {
                        username: username.to_string(),
                        password: password.to_string(),
                        step: 0,
                    },
                )
                .await
        }
        Mechanism::Command => client.login(username, password).await,
    };
    outcome.map_err(|(error, _client)| refusal(error))
}

/// RFC 4616: an authorisation identity nobody uses, the username, and the password, separated by
/// NULs. The crate does the base64.
struct PlainSasl {
    username: String,
    password: String,
}

impl Authenticator for PlainSasl {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8];
        out.extend_from_slice(self.username.as_bytes());
        out.push(0);
        out.extend_from_slice(self.password.as_bytes());
        out
    }
}

/// The non-standard but universally implemented SASL LOGIN: two challenges, username then
/// password. Counted rather than matched on the prompt text, because the prompt is not specified
/// and servers word it differently.
struct LoginSasl {
    username: String,
    password: String,
    step: u8,
}

impl Authenticator for LoginSasl {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Vec<u8> {
        self.step += 1;
        if self.step == 1 {
            self.username.as_bytes().to_vec()
        } else {
            self.password.as_bytes().to_vec()
        }
    }
}

/// RFC 2195: the username, a space, and HMAC-MD5 of the server's challenge keyed by the password,
/// in lowercase hex.
struct CramMd5 {
    username: String,
    password: String,
}

impl Authenticator for CramMd5 {
    type Response = Vec<u8>;

    fn process(&mut self, challenge: &[u8]) -> Vec<u8> {
        cram_md5_response(&self.username, &self.password, challenge).into_bytes()
    }
}

fn cram_md5_response(username: &str, password: &str, challenge: &[u8]) -> String {
    let digest = hmac_md5(password.as_bytes(), challenge);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{username} {hex}")
}

/// A login refusal, with the server's own words dug back out.
///
/// `async-imap` wraps a NO into `code: None, info: Some("...")`, and that sentence is where "use an
/// app password" comes from, which `imap::advice_for` turns into something to go and do. Losing it
/// inside a debug format would cost the one useful thing the server said.
fn refusal(error: ImapError) -> Refused {
    match error {
        ImapError::No(said) | ImapError::Bad(said) => Refused::Auth(said_by_server(&said)),
        ImapError::ConnectionLost => {
            Refused::Unreachable("the connection closed during login".to_string())
        }
        ImapError::Io(e) => Refused::Unreachable(e.to_string()),
        other => Refused::Other(other.to_string()),
    }
}

fn said_by_server(wrapped: &str) -> String {
    let Some(start) = wrapped.find("info: Some(\"") else {
        return wrapped.to_string();
    };
    let rest = &wrapped[start + "info: Some(\"".len()..];
    match rest.rfind("\")") {
        Some(end) if end > 0 => rest[..end].to_string(),
        _ => wrapped.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// MD5
// ---------------------------------------------------------------------------------------------

/// The sixty-four sine constants of RFC 1321, and the per-round rotations beside them.
#[rustfmt::skip]
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee,
    0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
    0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039,
    0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

#[rustfmt::skip]
const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
    5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn md5(input: &[u8]) -> [u8; 16] {
    let (mut a0, mut b0, mut c0, mut d0) =
        (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);

    let mut message = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_le_bytes());

    for block in message.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (index, word) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for round in 0..64 {
            let (mixed, index) = match round / 16 {
                0 => ((b & c) | (!b & d), round),
                1 => ((d & b) | (!d & c), (5 * round + 1) % 16),
                2 => (b ^ c ^ d, (3 * round + 5) % 16),
                _ => (c ^ (b | !d), (7 * round) % 16),
            };
            let mixed = mixed
                .wrapping_add(a)
                .wrapping_add(MD5_K[round])
                .wrapping_add(words[index]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(mixed.rotate_left(MD5_S[round]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

fn hmac_md5(key: &[u8], message: &[u8]) -> [u8; 16] {
    let mut padded = [0u8; 64];
    if key.len() > 64 {
        padded[..16].copy_from_slice(&md5(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(64 + message.len());
    let mut outer = Vec::with_capacity(64 + 16);
    for byte in padded {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(message);
    outer.extend_from_slice(&md5(&inner));
    md5(&outer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(listed: &[&str]) -> Vec<String> {
        listed.iter().map(|s| s.to_string()).collect()
    }

    fn printed(digest: [u8; 16]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn cram_md5_is_preferred_and_then_the_two_sasl_mechanisms_in_order() {
        assert_eq!(
            mechanism(&caps(&["IMAP4REV1", "AUTH=PLAIN", "AUTH=CRAM-MD5", "AUTH=LOGIN"])),
            Some(Mechanism::CramMd5)
        );
        assert_eq!(
            mechanism(&caps(&["IMAP4REV1", "AUTH=LOGIN", "AUTH=PLAIN"])),
            Some(Mechanism::Plain)
        );
        assert_eq!(
            mechanism(&caps(&["IMAP4REV1", "AUTH=LOGIN"])),
            Some(Mechanism::Login)
        );
    }

    #[test]
    fn a_server_with_no_sasl_at_all_gets_the_login_command() {
        assert_eq!(
            mechanism(&caps(&["IMAP4REV1", "IDLE"])),
            Some(Mechanism::Command)
        );
    }

    /// The one that matters: `LOGIN` must never be sent to a server that has forbidden it, and a
    /// server that forbids it and offers nothing else is one this app cannot use.
    #[test]
    fn logindisabled_is_obeyed() {
        assert_eq!(mechanism(&caps(&["IMAP4REV1", "LOGINDISABLED"])), None);
        assert_eq!(
            mechanism(&caps(&["IMAP4REV1", "LOGINDISABLED", "AUTH=PLAIN"])),
            Some(Mechanism::Plain)
        );
    }

    #[test]
    fn the_capability_names_are_compared_without_case() {
        assert_eq!(
            mechanism(&caps(&["imap4rev1", "auth=cram-md5"])),
            Some(Mechanism::CramMd5)
        );
        assert_eq!(mechanism(&caps(&["imap4rev1", "logindisabled"])), None);
    }

    #[test]
    fn a_capability_line_becomes_the_names_this_module_compares() {
        let listed = [
            async_imap::imap_proto::Capability::Imap4rev1,
            async_imap::imap_proto::Capability::Auth("cram-md5".into()),
            async_imap::imap_proto::Capability::Atom("logindisabled".into()),
        ];
        let names: Vec<String> = listed.iter().map(borrowed_capability).collect();
        assert_eq!(names, ["IMAP4REV1", "AUTH=CRAM-MD5", "LOGINDISABLED"]);
    }

    #[test]
    fn a_refusal_keeps_the_sentence_the_server_actually_wrote() {
        let wrapped = "code: None, info: Some(\"Application-specific password required\")";
        assert_eq!(
            said_by_server(wrapped),
            "Application-specific password required"
        );
        // Anything that is not in that shape is passed through rather than trimmed to nothing.
        assert_eq!(said_by_server("plain words"), "plain words");
        assert_eq!(
            said_by_server("code: Some(Alert), info: None"),
            "code: Some(Alert), info: None"
        );
    }

    #[test]
    fn a_no_response_becomes_an_auth_refusal_with_the_advice_still_readable() {
        let refused = refusal(ImapError::No(
            "code: None, info: Some(\"Please log in via your web browser\")".to_string(),
        ));
        match refused {
            Refused::Auth(said) => assert_eq!(said, "Please log in via your web browser"),
            other => panic!("expected an auth refusal, got {other:?}"),
        }
    }

    /// RFC 1321's own test suite. A wrong MD5 here is a login that fails on every server that
    /// prefers CRAM-MD5, so these are the vectors that make the hand written hash defensible.
    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        assert_eq!(printed(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(printed(md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(printed(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            printed(md5(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            printed(md5(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            printed(md5(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        // Long enough to need a second block, which is where a padding mistake shows up.
        assert_eq!(
            printed(md5(
                b"1234567890123456789012345678901234567890\
                  1234567890123456789012345678901234567890"
            )),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    /// RFC 2202, including the case where the key is longer than the block and has to be hashed
    /// first, which is the branch a CRAM-MD5 login with a long app password takes.
    #[test]
    fn hmac_md5_matches_the_rfc_2202_vectors() {
        assert_eq!(
            printed(hmac_md5(&[0x0b; 16], b"Hi There")),
            "9294727a3638bb1c13f48ef8158bfc9d"
        );
        assert_eq!(
            printed(hmac_md5(b"Jefe", b"what do ya want for nothing?")),
            "750c783e6ab0b503eaa86e310a5db738"
        );
        assert_eq!(
            printed(hmac_md5(&[0xaa; 16], &[0xdd; 50])),
            "56be34521d144c88dbb8c733f0e8b3f6"
        );
        assert_eq!(
            printed(hmac_md5(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "6b1ab7fe4bd7bf8f0b62e6ce61b9d0cd"
        );
    }

    /// The worked example in RFC 2195 itself, end to end.
    #[test]
    fn cram_md5_answers_the_rfc_2195_challenge() {
        assert_eq!(
            cram_md5_response(
                "tim",
                "tanstaaftanstaaf",
                b"<1896.697170952@postoffice.reston.mci.net>"
            ),
            "tim b913a602c7eda7a495b4e6e7334d3890"
        );
    }

    #[test]
    fn plain_is_the_rfc_4616_triple_and_login_is_two_answers_in_order() {
        let mut plain = PlainSasl {
            username: "you@example.com".to_string(),
            password: "secret".to_string(),
        };
        assert_eq!(plain.process(b""), b"\0you@example.com\0secret".to_vec());

        let mut login = LoginSasl {
            username: "you@example.com".to_string(),
            password: "secret".to_string(),
            step: 0,
        };
        assert_eq!(login.process(b"Username:"), b"you@example.com".to_vec());
        assert_eq!(login.process(b"Password:"), b"secret".to_vec());
    }
}
