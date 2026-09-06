// Sending: one SMTP submission, from the greeting to the last dot.
//
// OWNED BY THE SMTP PACKAGE.
//
// Only the submission is here. Putting the sent copy where the account expects to find it is an
// IMAP APPEND into whichever folder `folders` decided was Sent, and it belongs to the provider
// that has a mail connection open, not to the half that only ever talks to a relay.
//
// The conversation is written out here rather than driven by `mail-send`, and the reason is the
// certificate. `mail-send` owns its own TLS: the client builder makes a `TlsConnector` from the
// platform verifier, and `start_tls` is implemented only for a raw `TcpStream`. Letting it make
// its own connection means the loopback trust, the remembered fingerprints and the certificate
// question in `tls.rs` are all bypassed, and `tls.rs` is the one place in this app a certificate
// is decided about. That is worth more than the hundred lines of reply parsing it costs to keep.
//
// Two smaller things pushed the same way. `mail-send` picks a mechanism by preferring the highest
// bit `smtp-proto` handed it, which comes out PLAIN ahead of CRAM-MD5, and it refuses outright
// when nothing matches, where what is wanted here is Thunderbird's order and a gateway that asks
// for no credentials being allowed to have none. And its transparency procedure appends the end
// of data line unconditionally, so a message that already ends in CRLF gains a blank line at the
// bottom of every send.
//
// The one thing borrowed from it is the CRAM-MD5 digest, because MD5 is not a direct dependency
// of this crate and hand-writing a hash to avoid an import is the worse trade. `cram_md5` says
// why the mechanism number is written out and what keeps writing it out honest.
//
// `ServerConfig::auth` is not read here. Nothing builds an OAuth2 configuration yet, so every
// account that reaches this is a password one, and which mechanism carries that password is
// decided from what the server advertised rather than from the field.

use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::tls::{self, Refused, Stream};
use crate::dto::{Security, ServerConfig};

/// What this client calls itself in EHLO.
///
/// An address literal rather than a name, because there is no portable way to ask the machine for
/// its fully qualified name and a made up one is the thing HELO checks reject. The loopback
/// literal is well formed by RFC 5321 section 4.1.3 wherever it is read, and it says nothing
/// about the machine or the network it is on.
const EHLO_NAME: &str = "[127.0.0.1]";

/// How long one command may wait for its answer. `tls::CONNECT_TIMEOUT` already covers getting the
/// socket open; this covers everything after that, because a mail server that accepts a connection
/// and then says nothing is otherwise a spinner with no end.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the message itself gets, from the first byte written to the answer after the last dot.
/// Longer than the rest on purpose: the server has the whole message by then and a good many of
/// them scan it before saying yes.
const DATA_TIMEOUT: Duration = Duration::from_secs(300);

/// A reply this long is not a reply. Nothing legitimate comes close, and without a ceiling a
/// server that streams forever is a connection that reads forever.
const MAX_REPLY: usize = 64 * 1024;

// ---------------------------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------------------------

/// One answer from the server: the code it led with, and its own words with the codes taken off.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reply {
    code: u16,
    /// One entry per line of the reply, each without its code and separator.
    lines: Vec<String>,
}

impl Reply {
    /// What the server said. The numeric code is left off because what a person needs to read is
    /// the sentence, and for a mail server the sentence is usually the whole of the diagnosis.
    fn text(&self) -> String {
        let said = self.lines.join(" ");
        let said = said.trim();
        if said.is_empty() {
            format!("the server answered {}", self.code)
        } else {
            said.to_string()
        }
    }

    fn is_positive(&self) -> bool {
        (200..300).contains(&self.code)
    }
}

/// Takes one complete reply off the front of a buffer and says how many bytes it used, or `None`
/// when what is there so far is not yet a whole reply.
///
/// A reply is one or more lines all carrying the same code. A hyphen after the code means another
/// line follows and a space means that was the last, which is the only way to know a multi-line
/// reply has ended: there is no length and no terminator to look for.
///
/// A line that does not start with three digits ends the reply with a code of zero rather than
/// asking for more bytes, because a parser that waits for a code that is never coming waits until
/// the timeout on a server that is simply speaking something else.
fn take_reply(buffer: &[u8]) -> Option<(Reply, usize)> {
    let mut lines: Vec<String> = Vec::new();
    let mut code = 0u16;
    let mut used = 0usize;

    loop {
        let rest = buffer.get(used..)?;
        let newline = rest.iter().position(|byte| *byte == b'\n')?;
        let line = match rest[..newline].split_last() {
            Some((b'\r', head)) => head,
            _ => &rest[..newline],
        };
        used += newline + 1;

        let text = String::from_utf8_lossy(line);
        let digits = text.get(..3).and_then(|digits| digits.parse::<u16>().ok());
        if lines.is_empty() {
            code = digits.unwrap_or(0);
        }
        let more = digits.is_some() && text.as_bytes().get(3) == Some(&b'-');
        lines.push(text.get(4..).unwrap_or("").trim().to_string());

        if !more {
            return Some((Reply { code, lines }, used));
        }
    }
}

/// The extensions an EHLO answer advertised, uppercased.
///
/// The first line of the answer is the server naming itself and greeting, never an extension, so
/// it is dropped. Uppercasing here rather than at each reader is what lets everything downstream
/// compare against a literal: the keywords are case insensitive and servers differ.
fn capabilities(reply: &Reply) -> Vec<String> {
    reply
        .lines
        .iter()
        .skip(1)
        .map(|line| line.to_uppercase())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mechanism {
    CramMd5,
    Plain,
    Login,
}

/// The mechanisms the server offered, or `None` when it offered no AUTH line at all.
///
/// Expects the uppercased lines `capabilities` produces.
fn advertised_auth(capabilities: &[String]) -> Option<Vec<String>> {
    capabilities.iter().find_map(|line| {
        // `AUTH PLAIN LOGIN` is the form in the RFC. `AUTH=PLAIN LOGIN` is the form some Exchange
        // and Courier builds still emit, from a draft that predates the extension being
        // registered, and a client that reads only the first form sees those servers as offering
        // no authentication at all.
        let listed = line
            .strip_prefix("AUTH ")
            .or_else(|| line.strip_prefix("AUTH="))?;
        Some(listed.split_whitespace().map(str::to_string).collect())
    })
}

/// Which mechanism to authenticate with, or `None` to skip authenticating entirely.
///
/// The order is Thunderbird's. `None` is not a failure: a server that advertises no AUTH line is
/// a server that wants no credentials, which some gateways and some local bridges genuinely do,
/// and it is why the manual configuration sheet does not insist on an SMTP password.
fn choose_mechanism(capabilities: &[String]) -> Option<Mechanism> {
    let listed = advertised_auth(capabilities)?;
    let offered = |name: &str| listed.iter().any(|mechanism| mechanism == name);

    if offered("CRAM-MD5") {
        Some(Mechanism::CramMd5)
    } else if offered("PLAIN") {
        Some(Mechanism::Plain)
    } else if offered("LOGIN") {
        Some(Mechanism::Login)
    } else {
        // A list with none of the three in it is a list of things this client cannot do, such as
        // GSSAPI or XOAUTH2. PLAIN is tried anyway rather than refused here, because the server's
        // rejection carries the server's own words and "no mechanism in common" carries nothing
        // anybody can act on.
        Some(Mechanism::Plain)
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// SASL PLAIN's response: an empty authorisation identity, the username and the password, each
/// separated by a NUL. Sent as the initial response inside the AUTH command rather than after a
/// challenge, which every server that advertises PLAIN accepts and which saves a round trip.
fn plain_response(username: &str, password: &str) -> String {
    encode(format!("\0{username}\0{password}").as_bytes())
}

/// `smtp-proto`'s bit for CRAM-MD5, written out because `smtp-proto` is `mail-send`'s dependency
/// and not this crate's, so the constant it declares cannot be named from here.
const CRAM_MD5: u64 = 1 << 37;

/// The answer to a CRAM-MD5 challenge, which is the username and an HMAC-MD5 of the challenge
/// keyed by the password, base64 encoded.
///
/// The digest comes from `mail-send` because MD5 is not a direct dependency of this crate and
/// writing out a hash function to avoid an import is the worse of the two trades. What makes
/// writing out the mechanism number safe is the test below: it pins the pair to RFC 2195's own
/// worked example, so a future `smtp-proto` that renumbers its flags fails `cargo test` rather
/// than quietly sending the wrong digest.
fn cram_md5(username: &str, password: &str, challenge: &str) -> Result<String, Refused> {
    mail_send::Credentials::new(username, password)
        .encode(CRAM_MD5, challenge)
        .map_err(|e| Refused::Auth(e.to_string()))
}

// ---------------------------------------------------------------------------------------------
// The message on the wire
// ---------------------------------------------------------------------------------------------

/// CR and LF only ever appear together in a message, so a lone one of either becomes a pair.
///
/// `mail-builder` already produces CRLF and in principle this changes nothing, but a body that has
/// been anywhere else can carry a bare LF, and a server that treats it as the end of a line either
/// rejects the message or, worse, accepts a truncated one.
fn normalise_endings(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'\r' => {
                out.extend_from_slice(b"\r\n");
                // A CR that already has its LF consumes both, which is what stops an ordinary
                // CRLF from becoming CRLFLF and doubling every line in the message.
                index += if raw.get(index + 1) == Some(&b'\n') { 2 } else { 1 };
            }
            b'\n' => {
                out.extend_from_slice(b"\r\n");
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    out
}

/// The transparency procedure from RFC 5321 section 4.5.2: a line beginning with a period gets a
/// second one, so that the line ending the message is the only period that can stand alone.
///
/// Assumes line endings are already canonical, which is why `data_payload` normalises first.
fn stuff_dots(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 8);
    let mut at_line_start = true;
    for byte in raw {
        if at_line_start && *byte == b'.' {
            out.push(b'.');
        }
        out.push(*byte);
        at_line_start = *byte == b'\n';
    }
    out
}

/// Everything that goes on the wire between the DATA command and its answer.
fn data_payload(raw: &[u8]) -> Vec<u8> {
    let mut out = stuff_dots(&normalise_endings(raw));
    // The CRLF in front of the lone period belongs to the terminator rather than to the message,
    // so a body that already ends with one does not get a second. Adding one regardless is what
    // puts a blank line at the bottom of every message a client sends.
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

// ---------------------------------------------------------------------------------------------
// The conversation
// ---------------------------------------------------------------------------------------------

struct Conversation {
    stream: Stream,
    /// What has arrived and not yet been read as a reply. A reply can be split across reads and
    /// two replies can arrive in one, so the bytes outlive the call that fetched them.
    buffer: Vec<u8>,
    /// Set while a message body is on the wire and cleared once it has been answered for. It is
    /// the one moment in the conversation when anything else written to this socket would be read
    /// as a line of that message rather than as a command.
    mid_message: bool,
}

impl Conversation {
    fn new(stream: Stream) -> Conversation {
        Conversation {
            stream,
            buffer: Vec::new(),
            mid_message: false,
        }
    }

    async fn write(&mut self, bytes: &[u8], within: Duration) -> Result<(), Refused> {
        let sent = tokio::time::timeout(within, async {
            self.stream.write_all(bytes).await?;
            self.stream.flush().await
        });
        match sent.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(Refused::Unreachable(e.to_string())),
            Err(_) => Err(Refused::Unreachable(
                "the server stopped reading part way through the message".to_string(),
            )),
        }
    }

    async fn read_reply(&mut self, within: Duration) -> Result<Reply, Refused> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if let Some((reply, used)) = take_reply(&self.buffer) {
                self.buffer.drain(..used);
                return Ok(reply);
            }
            if self.buffer.len() > MAX_REPLY {
                return Err(Refused::Other(
                    "the server sent something that is not an SMTP reply".to_string(),
                ));
            }

            let mut chunk = [0u8; 2048];
            let read = tokio::time::timeout_at(deadline, self.stream.read(&mut chunk))
                .await
                .map_err(|_| {
                    Refused::Unreachable(format!(
                        "the server did not answer within {} seconds",
                        within.as_secs()
                    ))
                })?
                .map_err(|e| Refused::Unreachable(e.to_string()))?;
            if read == 0 {
                return Err(Refused::Unreachable(
                    "the server closed the connection".to_string(),
                ));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    async fn command(&mut self, line: &str) -> Result<Reply, Refused> {
        self.write(line.as_bytes(), REPLY_TIMEOUT).await?;
        self.read_reply(REPLY_TIMEOUT).await
    }

    /// EHLO, and the extensions it answered with.
    ///
    /// There is no HELO fallback. A server too old to understand EHLO is older than both SMTP AUTH
    /// and STARTTLS, so there is nothing this client could go on to do with it.
    async fn hello(&mut self) -> Result<Vec<String>, Refused> {
        let reply = self.command(&format!("EHLO {EHLO_NAME}\r\n")).await?;
        if !reply.is_positive() {
            return Err(Refused::Other(reply.text()));
        }
        Ok(capabilities(&reply))
    }

    /// Sends STARTTLS and hands the same socket to `tls::upgrade`.
    async fn start_tls(mut self, host: &str, port: u16) -> Result<Conversation, Refused> {
        let reply = self.command("STARTTLS\r\n").await?;
        if !reply.is_positive() {
            return Err(Refused::Other(reply.text()));
        }

        // Anything the server sent after answering STARTTLS arrived in the clear and would be read
        // as though it had arrived inside TLS, which is the plaintext command injection that
        // CVE-2011-0411 named. Nothing legitimate is ever there, so something being there is the
        // end of the connection rather than a thing to work around.
        if !self.buffer.is_empty() {
            return Err(Refused::Other(
                "the server sent data before the connection was encrypted".to_string(),
            ));
        }

        let socket = match self.stream {
            Stream::Plain(socket) => socket,
            Stream::Tls(_) => {
                return Err(Refused::Other(
                    "this connection is already encrypted".to_string(),
                ))
            }
        };
        Ok(Conversation::new(tls::upgrade(socket, host, port).await?))
    }

    async fn authenticate(
        &mut self,
        capabilities: &[String],
        username: &str,
        password: &str,
    ) -> Result<(), Refused> {
        let Some(mechanism) = choose_mechanism(capabilities) else {
            return Ok(());
        };

        // Every answer in this exchange that is not the one expected is the login being refused,
        // and it is returned with the server's own words on it. Those words are what `advice_for`
        // in `imap/mod.rs` reads to say things like "this account wants an app password".
        let reply = match mechanism {
            Mechanism::Plain => {
                let response = plain_response(username, password);
                self.command(&format!("AUTH PLAIN {response}\r\n")).await?
            }
            Mechanism::Login => {
                let reply = self.command("AUTH LOGIN\r\n").await?;
                if reply.code != 334 {
                    return Err(Refused::Auth(reply.text()));
                }
                let reply = self
                    .command(&format!("{}\r\n", encode(username.as_bytes())))
                    .await?;
                if reply.code != 334 {
                    return Err(Refused::Auth(reply.text()));
                }
                self.command(&format!("{}\r\n", encode(password.as_bytes())))
                    .await?
            }
            Mechanism::CramMd5 => {
                let reply = self.command("AUTH CRAM-MD5\r\n").await?;
                if reply.code != 334 {
                    return Err(Refused::Auth(reply.text()));
                }
                let response = cram_md5(username, password, &reply.text())?;
                self.command(&format!("{response}\r\n")).await?
            }
        };

        if reply.code == 235 {
            Ok(())
        } else {
            Err(Refused::Auth(reply.text()))
        }
    }

    /// The envelope and the message. The recipients are the ones passed in and never the ones in
    /// the headers, which is the whole of how Bcc reaches a server without reaching a reader.
    async fn deliver(&mut self, from: &str, to: &[String], raw: &[u8]) -> Result<(), Refused> {
        let reply = self.command(&format!("MAIL FROM:<{from}>\r\n")).await?;
        if !reply.is_positive() {
            return Err(Refused::Other(reply.text()));
        }

        for recipient in to {
            let reply = self.command(&format!("RCPT TO:<{recipient}>\r\n")).await?;
            // One refused recipient stops the send. A partial delivery is the worse outcome:
            // the app would report a failure that some of the recipients had already seen, and a
            // retry would send it to them twice.
            if !reply.is_positive() {
                return Err(Refused::Other(reply.text()));
            }
        }

        let reply = self.command("DATA\r\n").await?;
        if reply.code != 354 {
            return Err(Refused::Other(reply.text()));
        }

        // Anything that goes wrong from here until the answer arrives leaves this flag set, which
        // is what stops a QUIT being written into the middle of somebody's message.
        self.mid_message = true;
        self.write(&data_payload(raw), DATA_TIMEOUT).await?;
        let reply = self.read_reply(DATA_TIMEOUT).await?;
        self.mid_message = false;

        if !reply.is_positive() {
            return Err(Refused::Other(reply.text()));
        }
        Ok(())
    }
}

/// Connects the way the security says to, greets, upgrades if it has to, and logs in.
async fn connect(server: &ServerConfig, password: &str) -> Result<Conversation, Refused> {
    let stream = tls::connect(&server.host, server.port, server.security).await?;
    let mut talk = Conversation::new(stream);

    let greeting = talk.read_reply(REPLY_TIMEOUT).await?;
    // A 554 here is a server refusing the connection outright, usually because of where it came
    // from, and it says why in the greeting.
    if greeting.code != 220 {
        return Err(Refused::Other(greeting.text()));
    }

    let mut capabilities = talk.hello().await?;

    if matches!(server.security, Security::StartTls) {
        // Looked for only so that the refusal is a sentence somebody can act on. It is not a
        // condition on the upgrade in any sense that could be stripped: a configuration that says
        // StartTls never authenticates in the clear, whether the extension was advertised or not.
        if !capabilities.iter().any(|line| line == "STARTTLS") {
            return Err(Refused::Other(format!(
                "{} offers no STARTTLS on port {}, so the password would travel in the clear",
                server.host, server.port
            )));
        }
        talk = talk.start_tls(&server.host, server.port).await?;
        // The second EHLO is required by RFC 3207 and it is not a formality. The first list is
        // the one the server offers in the clear, and the AUTH line is usually only on the second.
        capabilities = talk.hello().await?;
    }

    talk.authenticate(&capabilities, &server.username, password)
        .await?;
    Ok(talk)
}

/// QUIT, and no interest in the answer. By the time this is reached the message has either been
/// accepted or it has not, and a server that hangs up rudely does not change which.
///
/// Nothing is said at all when a message was interrupted part way through. The socket is inside
/// the message at that point, so QUIT would be a line of it, and hanging up in silence is what
/// tells the server to throw away the half that arrived.
async fn goodbye(mut talk: Conversation) {
    if talk.mid_message {
        return;
    }
    let _ = talk.command("QUIT\r\n").await;
}

/// Connects and authenticates without sending anything, for the connect screen's test.
pub async fn check(server: &ServerConfig, password: &str) -> Result<(), Refused> {
    goodbye(connect(server, password).await?).await;
    Ok(())
}

/// One message, already built. The envelope recipients are given rather than parsed back out of
/// the headers, because Bcc must reach the server and must not reach the message.
pub async fn send(
    server: &ServerConfig,
    password: &str,
    from: &str,
    to: &[String],
    raw: &[u8],
) -> Result<(), Refused> {
    if to.is_empty() {
        return Err(Refused::Other("there is nobody to send this to".to_string()));
    }

    let mut talk = connect(server, password).await?;
    let sent = talk.deliver(from, to, raw).await;
    goodbye(talk).await;
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::AuthKind;
    use tokio::net::TcpListener;

    // -----------------------------------------------------------------------------------------
    // The message on the wire
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_line_that_begins_with_a_period_is_sent_with_two() {
        assert_eq!(stuff_dots(b".hidden\r\n"), b"..hidden\r\n");
        // The line that is only a period, which is the one that would end the message early.
        assert_eq!(stuff_dots(b"a\r\n.\r\nb\r\n"), b"a\r\n..\r\nb\r\n");
        // A line that already begins with two gets a third, because the server takes one off.
        assert_eq!(stuff_dots(b"..oops\r\n"), b"...oops\r\n");
        // A period anywhere but the start of a line is an ordinary character.
        assert_eq!(stuff_dots(b"see fig. 1\r\n"), b"see fig. 1\r\n");
    }

    #[test]
    fn the_payload_ends_with_one_terminator_however_the_message_ended() {
        assert_eq!(
            data_payload(b"Subject: hi\r\n\r\nbody\r\n"),
            b"Subject: hi\r\n\r\nbody\r\n.\r\n"
        );
        // No trailing newline, so the CRLF in front of the period has to be supplied.
        assert_eq!(
            data_payload(b"Subject: hi\r\n\r\nbody"),
            b"Subject: hi\r\n\r\nbody\r\n.\r\n"
        );
        // And a message that ended with one does not gain a blank line at the bottom.
        let payload = data_payload(b"body\r\n");
        assert!(!payload.ends_with(b"\r\n\r\n.\r\n"));
    }

    #[test]
    fn a_bare_newline_becomes_a_pair_and_a_pair_stays_one_pair() {
        assert_eq!(normalise_endings(b"a\nb\n"), b"a\r\nb\r\n");
        assert_eq!(normalise_endings(b"a\r\nb\r\n"), b"a\r\nb\r\n");
        // The mixture, which is what a body that has been through something else looks like.
        assert_eq!(normalise_endings(b"a\r\nb\nc\r\n"), b"a\r\nb\r\nc\r\n");
        // A lone CR is the other half of the same mistake.
        assert_eq!(normalise_endings(b"a\rb"), b"a\r\nb");
        assert_eq!(normalise_endings(b""), b"");
    }

    #[test]
    fn normalising_runs_before_stuffing_so_a_period_after_a_bare_newline_is_still_found() {
        assert_eq!(data_payload(b"a\n.\nb"), b"a\r\n..\r\nb\r\n.\r\n");
    }

    // -----------------------------------------------------------------------------------------
    // Replies
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_reply_is_only_complete_once_a_line_answers_with_a_space() {
        assert!(take_reply(b"220 mail.example.com ESMTP").is_none());
        assert!(take_reply(b"250-mail.example.com\r\n250-SIZE 100\r\n").is_none());

        let (reply, used) = take_reply(b"220 mail.example.com ESMTP\r\n").expect("one reply");
        assert_eq!(reply.code, 220);
        assert_eq!(reply.text(), "mail.example.com ESMTP");
        assert_eq!(used, 28);

        // Two replies in one read, which is what a server that answers quickly looks like.
        let both = b"354 go ahead\r\n250 2.0.0 queued\r\n";
        let (first, used) = take_reply(both).expect("the first reply");
        assert_eq!(first.code, 354);
        let (second, _) = take_reply(&both[used..]).expect("the second reply");
        assert_eq!(second.code, 250);
        assert_eq!(second.text(), "2.0.0 queued");
    }

    #[test]
    fn a_line_that_is_not_a_reply_ends_the_reply_rather_than_asking_for_more() {
        let (reply, _) = take_reply(b"<html>go away</html>\r\n").expect("something");
        assert_eq!(reply.code, 0);
        assert!(!reply.is_positive());
    }

    /// An EHLO answer built the way a real one arrives, so the continuation form and the final
    /// line are both exercised on the way to every capability test below.
    fn advertised(extensions: &[&str]) -> Vec<String> {
        let mut raw = String::from("250-mail.example.com at your service\r\n");
        for extension in extensions {
            raw.push_str(&format!("250-{extension}\r\n"));
        }
        raw.push_str("250 HELP\r\n");
        let (reply, used) = take_reply(raw.as_bytes()).expect("a complete EHLO answer");
        assert_eq!(used, raw.len());
        capabilities(&reply)
    }

    #[test]
    fn an_ehlo_answer_reads_as_its_extensions_without_the_greeting() {
        let found = advertised(&["SIZE 35882577", "8BITMIME", "STARTTLS", "AUTH LOGIN PLAIN"]);
        assert_eq!(
            found,
            vec![
                "SIZE 35882577".to_string(),
                "8BITMIME".to_string(),
                "STARTTLS".to_string(),
                "AUTH LOGIN PLAIN".to_string(),
                "HELP".to_string(),
            ]
        );
        // The server naming itself is not an extension, and a client that reads it as one goes
        // looking for a host called STARTTLS.
        assert!(!found.iter().any(|line| line.contains("AT YOUR SERVICE")));
    }

    // -----------------------------------------------------------------------------------------
    // Authentication
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_mechanism_is_the_first_of_thunderbirds_three_the_server_offered() {
        assert_eq!(
            choose_mechanism(&advertised(&["AUTH LOGIN PLAIN CRAM-MD5"])),
            Some(Mechanism::CramMd5)
        );
        assert_eq!(
            choose_mechanism(&advertised(&["AUTH LOGIN PLAIN"])),
            Some(Mechanism::Plain)
        );
        assert_eq!(
            choose_mechanism(&advertised(&["AUTH LOGIN"])),
            Some(Mechanism::Login)
        );
        // Keywords are case insensitive and servers do disagree about them.
        assert_eq!(
            choose_mechanism(&advertised(&["auth cram-md5 plain"])),
            Some(Mechanism::CramMd5)
        );
        // The form old Exchange still emits alongside the one in the RFC.
        assert_eq!(
            choose_mechanism(&advertised(&["AUTH=LOGIN PLAIN"])),
            Some(Mechanism::Plain)
        );
        // Nothing in common, so PLAIN is tried and the server gets to say why in its own words.
        assert_eq!(
            choose_mechanism(&advertised(&["AUTH GSSAPI XOAUTH2"])),
            Some(Mechanism::Plain)
        );
        // And no AUTH line at all means no credentials, which some gateways genuinely want.
        assert_eq!(
            choose_mechanism(&advertised(&["SIZE 35882577", "8BITMIME"])),
            None
        );
    }

    #[test]
    fn the_plain_response_is_the_two_nul_separated_fields_and_nothing_else() {
        // RFC 4616's shape, checked against the encoding of "\0tim\0tanstaaftanstaaf".
        assert_eq!(
            plain_response("tim", "tanstaaftanstaaf"),
            "AHRpbQB0YW5zdGFhZnRhbnN0YWFm"
        );
        // The authorisation identity is empty, so the very first byte of the plaintext is a NUL.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(plain_response("me@example.com", "pw"))
            .expect("valid base64");
        assert_eq!(decoded, b"\0me@example.com\0pw");
    }

    #[test]
    fn the_cram_md5_mechanism_number_still_means_cram_md5() {
        // RFC 2195's own worked example. This test is what makes writing the mechanism number out
        // safe: `smtp-proto` is not this crate's dependency so its constant cannot be named, and
        // a release that renumbers the flags stops matching this vector rather than quietly
        // sending a digest computed the wrong way.
        assert_eq!(
            cram_md5(
                "tim",
                "tanstaaftanstaaf",
                "PDE4OTYuNjk3MTcwOTUyQHBvc3RvZmZpY2UucmVzdG9uLm1jaS5uZXQ+"
            )
            .expect("a response"),
            "dGltIGI5MTNhNjAyYzdlZGE3YTQ5NWI0ZTZlNzMzNGQzODkw"
        );
    }

    // -----------------------------------------------------------------------------------------
    // The whole conversation, against a listener this test owns
    // -----------------------------------------------------------------------------------------

    /// An SMTP server that answers from a script and keeps everything it was told.
    ///
    /// It knows only one thing about the protocol, which is that DATA is followed by a body rather
    /// than a command, so the body does not consume the script. Everything else is turn taking.
    async fn scripted(listener: TcpListener, replies: Vec<String>) -> Vec<u8> {
        let (mut socket, _) = listener.accept().await.expect("a connection");
        let mut heard: Vec<u8> = Vec::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 1024];
        let mut next = 0;
        let mut in_body = false;

        // A server that runs off the end of its script keeps agreeing, so a test only has to
        // write down the answers it cares about.
        let say = |next: &mut usize| -> String {
            let reply = replies
                .get(*next)
                .cloned()
                .unwrap_or_else(|| "250 OK\r\n".to_string());
            *next += 1;
            reply
        };

        let greeting = say(&mut next);
        socket
            .write_all(greeting.as_bytes())
            .await
            .expect("a greeting");

        loop {
            let read = socket.read(&mut chunk).await.expect("a read");
            if read == 0 {
                break;
            }
            heard.extend_from_slice(&chunk[..read]);
            pending.extend_from_slice(&chunk[..read]);

            while let Some(at) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=at).collect();
                let line = String::from_utf8_lossy(&line).trim_end().to_string();

                if in_body {
                    if line != "." {
                        continue;
                    }
                    in_body = false;
                } else if line.eq_ignore_ascii_case("DATA") {
                    in_body = true;
                }

                let reply = say(&mut next);
                socket.write_all(reply.as_bytes()).await.expect("a reply");
                if line.eq_ignore_ascii_case("QUIT") {
                    return heard;
                }
            }
        }
        heard
    }

    #[test]
    fn a_whole_send_reaches_a_server_with_the_envelope_it_was_given_and_a_stuffed_body() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
            let port = listener.local_addr().expect("an address").port();
            let server = tauri::async_runtime::spawn(scripted(
                listener,
                vec![
                    "220 fake.example.com ESMTP\r\n".to_string(),
                    "250-fake.example.com\r\n250-SIZE 1000000\r\n250 AUTH LOGIN PLAIN\r\n"
                        .to_string(),
                    "235 2.7.0 accepted\r\n".to_string(),
                    "250 2.1.0 sender ok\r\n".to_string(),
                    "250 2.1.5 recipient ok\r\n".to_string(),
                    "250 2.1.5 recipient ok\r\n".to_string(),
                    "354 go ahead\r\n".to_string(),
                    "250 2.0.0 queued as ABC123\r\n".to_string(),
                    "221 2.0.0 bye\r\n".to_string(),
                ],
            ));

            let config = ServerConfig {
                host: "127.0.0.1".to_string(),
                port,
                security: Security::Plain,
                auth: AuthKind::Password,
                username: "tim".to_string(),
            };
            send(
                &config,
                "tanstaaftanstaaf",
                "me@example.com",
                &["to@example.com".to_string(), "bcc@example.com".to_string()],
                b"Subject: hi\r\n\r\n.hidden\r\nsee fig. 1\r\n",
            )
            .await
            .expect("a send");

            let heard = String::from_utf8(server.await.expect("the server")).expect("utf8");

            assert!(heard.contains("EHLO [127.0.0.1]\r\n"), "{heard}");
            // PLAIN was chosen over LOGIN and sent as the initial response, in one command.
            assert!(
                heard.contains("AUTH PLAIN AHRpbQB0YW5zdGFhZnRhbnN0YWFm\r\n"),
                "{heard}"
            );
            // The envelope is the list that was passed in, so the Bcc recipient is on the wire
            // ahead of DATA and nowhere at all after it. That split is the whole point of taking
            // the recipients as an argument rather than reading them back out of the headers.
            let (envelope, body) = heard.split_once("DATA\r\n").expect("a DATA command");
            assert!(envelope.contains("MAIL FROM:<me@example.com>\r\n"), "{heard}");
            assert!(envelope.contains("RCPT TO:<to@example.com>\r\n"), "{heard}");
            assert!(envelope.contains("RCPT TO:<bcc@example.com>\r\n"), "{heard}");
            assert!(!body.contains("bcc@example.com"), "{heard}");
            // And the body arrived stuffed and terminated.
            assert!(body.contains("\r\n..hidden\r\nsee fig. 1\r\n.\r\n"), "{heard}");
        });
    }

    #[test]
    fn a_refused_login_comes_back_as_the_servers_own_words() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
            let port = listener.local_addr().expect("an address").port();
            let server = tauri::async_runtime::spawn(scripted(
                listener,
                vec![
                    "220 fake.example.com ESMTP\r\n".to_string(),
                    "250-fake.example.com\r\n250 AUTH PLAIN\r\n".to_string(),
                    "535-5.7.8 Username and Password not accepted.\r\n\
                     535 5.7.8 Application-specific password required.\r\n"
                        .to_string(),
                ],
            ));

            let config = ServerConfig {
                host: "127.0.0.1".to_string(),
                port,
                security: Security::Plain,
                auth: AuthKind::Password,
                username: "tim".to_string(),
            };
            let refused = check(&config, "wrong").await.expect_err("a refusal");

            // `imap::advice_for` reads this text to say what to do about it, so it has to be the
            // server's sentence and not a sentence of ours.
            match refused {
                Refused::Auth(said) => {
                    assert!(said.contains("Application-specific password required"), "{said}")
                }
                other => panic!("expected an authentication refusal, got {other:?}"),
            }
            let _ = server.await;
        });
    }

    /// One real message through a real server. Off by default because it needs both, and run with
    /// the account's own details in the environment:
    ///
    ///     SMTP_HOST=smtp.example.com SMTP_PORT=587 SMTP_USER=me@example.com \
    ///     SMTP_PASS=secret SMTP_TO=me@example.com \
    ///     cargo test --lib imap::smtp::tests::a_real_server -- --ignored --nocapture
    #[test]
    #[ignore]
    fn a_real_server() {
        let host = std::env::var("SMTP_HOST").expect("SMTP_HOST");
        let port: u16 = std::env::var("SMTP_PORT")
            .expect("SMTP_PORT")
            .parse()
            .expect("a port");
        let username = std::env::var("SMTP_USER").expect("SMTP_USER");
        let password = std::env::var("SMTP_PASS").expect("SMTP_PASS");
        let to = std::env::var("SMTP_TO").expect("SMTP_TO");

        let config = ServerConfig {
            host,
            port,
            security: if port == 465 {
                Security::Tls
            } else {
                Security::StartTls
            },
            auth: AuthKind::Password,
            username: username.clone(),
        };

        tauri::async_runtime::block_on(async {
            check(&config, &password).await.expect("a login");
            let raw = format!(
                "From: <{username}>\r\nTo: <{to}>\r\nSubject: margin mail smtp test\r\n\r\n\
                 Sent by the ignored test in imap/smtp.rs.\r\n"
            );
            send(&config, &password, &username, &[to], raw.as_bytes())
                .await
                .expect("a send");
        });
    }
}
