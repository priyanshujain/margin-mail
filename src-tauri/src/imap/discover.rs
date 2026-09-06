// Turning an email address into a pair of servers.
//
// OWNED BY THE DISCOVERY PACKAGE. Nothing outside this file may be edited to make it work.
//
// This is Thunderbird's ladder, rung for rung, because it is the only one that has been run against
// the whole of the internet's mail for fifteen years and every deviation from it is a domain that
// stops working. The four rungs are, in order of how much the answer is worth: a configuration the
// provider publishes about itself, a configuration the community publishes about the provider, the
// same two asked again about whoever actually receives the domain's mail, and finally a guess made
// by opening sockets.
//
// The rungs run at the same time and are resolved in that order, which is Thunderbird's
// `promiseFirstSuccessful`. Running them in sequence would mean that the common case, a domain in
// the central database, waits out four HTTP timeouts against hosts that were never going to answer
// before anybody sees a form. Running them concurrently and taking them in priority order costs the
// slowest rung above the winner and nothing else, and the losers are aborted the moment an answer
// exists. That is also what "the probe is only reached when the first three came up empty" means in
// practice: its sockets are opened, and its answer is thrown away unread unless nothing better
// arrived.
//
// Nothing here sends a credential. The probe reads a greeting and asks for capabilities, which is
// all that is needed to tell a mail server from a port that merely accepts connections, and a
// password offered to a host that was guessed at is a password given to whoever registered the
// guess.

use std::sync::LazyLock;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::tls::{self, Stream};
use crate::dto::{AuthKind, MailConfig, Security, ServerConfig};

/// Per URL on the provider-hosted rung. Short because there are four of them and most domains have
/// none: the usual outcome is four hosts that do not resolve, and a person is waiting.
const HOSTED_TIMEOUT: Duration = Duration::from_secs(5);

/// The central database is one request to one host that is always there, so it can afford longer.
const ISPDB_TIMEOUT: Duration = Duration::from_secs(10);

/// The gap between one probe connection and the next. Eighteen simultaneous SYNs from one address
/// is a shape that small mail servers rate limit, and the stagger costs under two seconds while
/// making the traffic look like a client rather than a scan.
const STAGGER: Duration = Duration::from_millis(100);

/// A whole probe conversation: connect, greeting, one command, one answer. `tls::connect` already
/// bounds its own half of this, so the outer budget is what stops a host that completes the
/// handshake and then says nothing from holding the rung open.
const CONVERSATION: Duration = tls::CONNECT_TIMEOUT;

/// Enough of a greeting and a capability list to decide on. A server that has not said what it can
/// do inside eight kilobytes is not one this is going to understand.
const MOST: usize = 8 * 1024;

// ---------------------------------------------------------------------------------------------
// The ladder
// ---------------------------------------------------------------------------------------------

/// The ladder. Returns `None` when every rung came up empty, which is not a failure: it means the
/// manual sheet is the next screen.
pub async fn discover(email: &str) -> Option<MailConfig> {
    let email = email.trim().to_string();
    let domain = domain_of(&email)?;

    // Spawned rather than joined so that the losers can be aborted. Dropping a `JoinHandle` detaches
    // the task instead of cancelling it, which on the probe rung would leave eighteen sockets open
    // behind an account that has already been set up.
    let mut rungs = vec![
        tokio::spawn(hosted(domain.clone(), email.clone(), Reach::Anywhere)),
        tokio::spawn(ispdb(domain.clone(), email.clone(), "ispdb")),
        tokio::spawn(from_mx(domain.clone(), email.clone())),
        tokio::spawn(probe(domain, email)),
    ];

    let mut answer = None;
    for index in 0..rungs.len() {
        // Awaited in declared order, so a slow high-priority rung is waited out rather than beaten
        // by a fast low-priority one. The order is the whole point of the ladder.
        if let Ok(Some(found)) = (&mut rungs[index]).await {
            answer = Some(found);
            break;
        }
    }
    for rung in rungs {
        rung.abort();
    }
    answer
}

/// The domain half of an address, lowercased and checked.
///
/// The check is not politeness. This value is pasted into a URL path and into a hostname on three of
/// the four rungs, so an address whose domain carries a slash or a question mark would be an address
/// that chooses which host gets asked.
fn domain_of(email: &str) -> Option<String> {
    let (local, domain) = email.rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    let domain = domain.trim().trim_end_matches('.').to_lowercase();
    is_hostname(&domain).then_some(domain)
}

fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

// ---------------------------------------------------------------------------------------------
// Rung one: what the provider publishes about itself
// ---------------------------------------------------------------------------------------------

/// Whether the plain-HTTP variants are on the table.
///
/// They are for the domain the person typed, because most hosters serve this file over HTTP only
/// and Thunderbird has always tried both. They are not for a domain derived from an MX record,
/// where the name was inferred rather than given and an unencrypted answer from an inferred host is
/// two guesses stacked on each other.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Anywhere,
    HttpsOnly,
}

/// The four URLs, in the order Thunderbird tries them. The subdomain comes before the well-known
/// path at each protocol because a hoster who runs `autoconfig.` is answering deliberately, while
/// `/.well-known/` on the main site is as likely to be a catch-all that returns the front page.
fn hosted_urls(domain: &str, email: &str, reach: Reach) -> Vec<String> {
    // The MX rung asks over HTTPS and drops the well-known path with it: a host reached by
    // inference should be asked at the one address that exists only to answer this question.
    let urls = match reach {
        Reach::HttpsOnly => vec![format!("https://autoconfig.{domain}/mail/config-v1.1.xml")],
        Reach::Anywhere => vec![
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
            format!("http://autoconfig.{domain}/mail/config-v1.1.xml"),
            format!("http://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        ],
    };
    urls.into_iter()
        .filter_map(|url| {
            let mut url = url::Url::parse(&url).ok()?;
            // The address is passed so a hoster that serves several brands from one file can answer
            // for the right one. Appended through the URL crate rather than by formatting, because
            // an address may contain a `+` or a `&` and a hand-built query string would split there.
            url.query_pairs_mut().append_pair("emailaddress", email);
            Some(url.to_string())
        })
        .collect()
}

async fn hosted(domain: String, email: String, reach: Reach) -> Option<MailConfig> {
    for url in hosted_urls(&domain, &email, reach) {
        let Some(body) = fetch(&url, HOSTED_TIMEOUT).await else {
            continue;
        };
        // A 200 that is not a configuration is the normal failure here, not an exceptional one:
        // catch-all hosting answers every path with the front page. The parse is the real test.
        if let Some(found) = parse(&body, &email, "autoconfig") {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Rung two: what the community publishes about the provider
// ---------------------------------------------------------------------------------------------

async fn ispdb(domain: String, email: String, source: &str) -> Option<MailConfig> {
    let url = format!("https://autoconfig.thunderbird.net/v1.1/{domain}");
    let body = fetch(&url, ISPDB_TIMEOUT).await?;
    parse(&body, &email, source)
}

// ---------------------------------------------------------------------------------------------
// Rung three: who actually receives this domain's mail
// ---------------------------------------------------------------------------------------------

/// The rung that recognises a vanity domain. A company whose address is `@northgate.example` and
/// whose mail is delivered to `aspmx.l.google.com` has published nothing about itself and is not in
/// the database, but Google is in both, so the question is asked again about Google.
async fn from_mx(domain: String, email: String) -> Option<MailConfig> {
    let exchange = lowest_mx(&domain).await?;
    for candidate in mx_domains(&exchange) {
        // Asking Google about a domain Google already answers for would be the first two rungs run
        // twice, and the second run would take just as long to fail.
        if candidate == domain || !is_hostname(&candidate) {
            continue;
        }
        if let Some(found) = hosted(candidate.clone(), email.clone(), Reach::HttpsOnly).await {
            return Some(MailConfig {
                source: "mx".to_string(),
                ..found
            });
        }
        if let Some(found) = ispdb(candidate, email.clone(), "mx").await {
            return Some(found);
        }
    }
    None
}

/// The exchange with the lowest preference, which is the host the domain's mail is actually offered
/// to first and therefore the one that says who runs the mailbox.
async fn lowest_mx(domain: &str) -> Option<String> {
    use hickory_resolver::proto::rr::{RData, RecordType};
    use hickory_resolver::TokioResolver;

    let resolver = TokioResolver::builder_tokio().ok()?.build().ok()?;
    let answer = resolver.lookup(domain, RecordType::MX).await.ok()?;
    let mut exchanges: Vec<(u16, String)> = answer
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::MX(mx) => Some((
                mx.preference,
                mx.exchange.to_string().trim_end_matches('.').to_lowercase(),
            )),
            _ => None,
        })
        .filter(|(_, host)| !host.is_empty())
        .collect();
    exchanges.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    exchanges.into_iter().next().map(|(_, host)| host)
}

/// The two domains worth asking about, given an exchange hostname.
///
/// The base domain is the one that identifies the operator: `google.com` out of `aspmx.l.google.com`
/// is what both the hosted rung and the database will answer for. The next label up is kept as well
/// because some operators publish per-brand rather than per-company, and `messagingengine.com` is
/// the case that motivates it: Fastmail's own file lives on `fastmail.com` while the exchange is
/// `in1-smtp.messagingengine.com`, so the wider net is worth one extra request.
fn mx_domains(exchange: &str) -> Vec<String> {
    let host = exchange.trim().trim_end_matches('.').to_lowercase();
    let Some(base) = base_domain(&host) else {
        return Vec::new();
    };
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    let depth = base.split('.').count();

    let mut out = vec![base];
    if labels.len() > depth {
        out.push(labels[labels.len() - depth - 1..].join("."));
    }
    out
}

/// The suffixes under which registrations happen two labels deep, so that `bbc.co.uk` is a
/// registrable domain and `co.uk` is not.
///
/// This is a list rather than a public suffix list on purpose. The real list is nine thousand lines
/// that go stale, it would be a new dependency and a new update problem, and the cost of being
/// wrong here is bounded: a domain reduced one label too far asks the wrong host a question and gets
/// a 404, and a domain reduced one label too few asks a host that does not exist. Neither is a wrong
/// answer, only a rung that does not fire, and the probe is still under it.
const TWO_LEVEL_SUFFIXES: &[&str] = &[
    "ac.at", "ac.il", "ac.in", "ac.jp", "ac.kr", "ac.nz", "ac.uk", "ac.za", "co.id", "co.il",
    "co.in", "co.jp", "co.kr", "co.nz", "co.th", "co.uk", "co.za", "com.ar", "com.au", "com.br",
    "com.cn", "com.co", "com.eg", "com.es", "com.hk", "com.mx", "com.my", "com.ng", "com.pe",
    "com.ph", "com.pk", "com.pl", "com.sa", "com.sg", "com.tr", "com.tw", "com.ua", "com.uy",
    "com.vn", "edu.au", "edu.cn", "edu.hk", "edu.in", "edu.sg", "gov.au", "gov.br", "gov.cn",
    "gov.in", "gov.uk", "govt.nz", "ne.jp", "net.au", "net.br", "net.cn", "net.in", "net.nz",
    "net.uk", "or.jp", "or.kr", "org.au", "org.br", "org.cn", "org.in", "org.nz", "org.uk",
    "org.za", "sch.uk",
];

/// The registrable part of a hostname: the last two labels, or the last three when the last two are
/// a suffix people register under rather than a domain anybody owns.
fn base_domain(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_lowercase();
    let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
    if labels.len() < 2 {
        return None;
    }
    let last_two = labels[labels.len() - 2..].join(".");
    if labels.len() > 2 && TWO_LEVEL_SUFFIXES.contains(&last_two.as_str()) {
        return Some(labels[labels.len() - 3..].join("."));
    }
    Some(last_two)
}

// ---------------------------------------------------------------------------------------------
// Rung four: the guess
// ---------------------------------------------------------------------------------------------

/// Which half of the account a candidate belongs to, and therefore which words are spoken at it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Leg {
    Imap,
    Smtp,
}

/// One host and port to try, and how the socket would be protected. An empty prefix means the
/// domain itself, which is how Posteo and a great many small hosts are reached.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Candidate {
    prefix: &'static str,
    port: u16,
    security: Security,
}

const fn at(prefix: &'static str, port: u16, security: Security) -> Candidate {
    Candidate {
        prefix,
        port,
        security,
    }
}

impl Candidate {
    fn host(&self, domain: &str) -> String {
        if self.prefix.is_empty() {
            domain.to_string()
        } else {
            format!("{}.{}", self.prefix, domain)
        }
    }
}

/// The order is Thunderbird's `sortTriesByPreference` and it says two things at once: implicit TLS
/// beats an upgrade beats nothing, and inside each of those the conventional name beats the domain
/// itself. POP is not here and never will be, so a host that offers both is set up as the IMAP
/// account it also is rather than as the lesser protocol that happened to answer first.
const INCOMING: &[Candidate] = &[
    at("imap", 993, Security::Tls),
    at("mail", 993, Security::Tls),
    at("", 993, Security::Tls),
    at("imap", 143, Security::StartTls),
    at("mail", 143, Security::StartTls),
    at("", 143, Security::StartTls),
    at("imap", 143, Security::Plain),
    at("mail", 143, Security::Plain),
    at("", 143, Security::Plain),
];

const OUTGOING: &[Candidate] = &[
    at("smtp", 465, Security::Tls),
    at("mail", 465, Security::Tls),
    at("", 465, Security::Tls),
    at("smtp", 587, Security::StartTls),
    at("mail", 587, Security::StartTls),
    at("", 587, Security::StartTls),
    at("smtp", 587, Security::Plain),
    at("mail", 587, Security::Plain),
    at("", 587, Security::Plain),
];

/// What is known about one candidate so far. `Pending` is the state that makes the decision a
/// function rather than a race: a lower-preference host answering first does not win while a
/// better one is still being waited for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Pending,
    Failed,
    Answered,
}

/// The winner, or nothing yet. Walks in preference order and stops at the first entry that is not a
/// definite failure: an answer there is the winner, and anything still pending means the question
/// cannot be settled without waiting for it.
fn first_answer<T>(tried: &[(T, Outcome)]) -> Option<&T> {
    for (candidate, outcome) in tried {
        match outcome {
            Outcome::Failed => continue,
            Outcome::Answered => return Some(candidate),
            Outcome::Pending => return None,
        }
    }
    None
}

async fn probe(domain: String, email: String) -> Option<MailConfig> {
    let incoming = tokio::spawn(probe_leg(domain.clone(), INCOMING, Leg::Imap));
    let outgoing = tokio::spawn(probe_leg(domain.clone(), OUTGOING, Leg::Smtp));

    // Both are awaited even when the first came up empty, because abandoning a handle detaches its
    // task rather than stopping it and the sockets would outlive the answer.
    let imap = incoming.await.ok().flatten();
    let smtp = outgoing.await.ok().flatten();

    // Half an answer is not an answer. A mailbox that can be read and not written to is a worse
    // starting point than the manual sheet, where at least both halves are asked about.
    Some(MailConfig {
        imap: server(&imap?, &domain, &email),
        smtp: server(&smtp?, &domain, &email),
        source: "probe".to_string(),
        display_name: None,
    })
}

fn server(candidate: &Candidate, domain: &str, email: &str) -> ServerConfig {
    ServerConfig {
        host: candidate.host(domain),
        port: candidate.port,
        security: candidate.security,
        // A guessed server is a password server. Nothing that has to be guessed at has an OAuth
        // client registered for this app, so there is no other value this could take.
        auth: AuthKind::Password,
        username: email.to_string(),
    }
}

async fn probe_leg(domain: String, table: &'static [Candidate], leg: Leg) -> Option<Candidate> {
    let (told, mut hears) = tokio::sync::mpsc::channel(table.len());
    let mut running = Vec::with_capacity(table.len());
    for (index, candidate) in table.iter().enumerate() {
        let host = candidate.host(&domain);
        let candidate = *candidate;
        let told = told.clone();
        running.push(tokio::spawn(async move {
            tokio::time::sleep(STAGGER * index as u32).await;
            let answered = speaks(&host, candidate.port, candidate.security, leg).await;
            let _ = told.send((index, answered)).await;
        }));
    }
    // The last sender in this scope, without which the loop below would never see the channel close
    // and would hang after the final task reported.
    drop(told);

    let mut tried: Vec<(Candidate, Outcome)> = table
        .iter()
        .map(|candidate| (*candidate, Outcome::Pending))
        .collect();
    let mut found = None;
    while let Some((index, answered)) = hears.recv().await {
        tried[index].1 = if answered {
            Outcome::Answered
        } else {
            Outcome::Failed
        };
        // Checked after every report rather than at the end, so a domain whose `imap.` host answers
        // on 993 does not wait out the eight worse candidates before the screen fills in.
        if let Some(winner) = first_answer(&tried) {
            found = Some(*winner);
            break;
        }
    }
    for task in running {
        task.abort();
    }
    found
}

/// Whether something on this host and port is the mail server it would have to be.
///
/// Deliberately not a login. Data arriving at all is most of the answer, because a port that is
/// open but silent is a load balancer and not a mailbox. The exception is STARTTLS, where an
/// upgrade that is not advertised is one that will not happen, and setting an account up to send a
/// password in the clear because a port was open is the one outcome worth writing a check for.
async fn speaks(host: &str, port: u16, security: Security, leg: Leg) -> bool {
    match tokio::time::timeout(CONVERSATION, converse(host, port, security, leg)).await {
        Ok(Some(said)) => match security {
            Security::StartTls => said.to_uppercase().contains("STARTTLS"),
            Security::Tls | Security::Plain => !said.is_empty(),
        },
        _ => false,
    }
}

async fn converse(host: &str, port: u16, security: Security, leg: Leg) -> Option<String> {
    // `connect` hands back an unencrypted socket for `StartTls`, which is exactly what is wanted:
    // the upgrade is never performed here, only looked for in what the server says it can do.
    let mut socket = tls::connect(host, port, security).await.ok()?;

    // The greeting is unprompted and comes first in both protocols. A host that accepts the
    // connection and then says nothing is not answered by any further question.
    read_until(&mut socket, |seen| seen.contains('\n')).await?;

    match leg {
        Leg::Imap => {
            socket.write_all(b"1 CAPABILITY\r\n").await.ok()?;
            let said = read_until(&mut socket, |seen| {
                seen.lines().any(|line| line.starts_with("1 "))
            })
            .await;
            // Sent without waiting for its answer. It is a courtesy to the server's logs rather
            // than part of the question, and the connection is about to be dropped either way.
            let _ = socket.write_all(b"2 LOGOUT\r\n").await;
            said
        }
        Leg::Smtp => {
            // A name rather than the machine's own, because the machine's own is usually a
            // residential PTR record and several servers refuse an EHLO that does not resolve.
            socket.write_all(b"EHLO margin.local\r\n").await.ok()?;
            let said = read_until(&mut socket, |seen| seen.lines().any(last_smtp_line)).await;
            let _ = socket.write_all(b"QUIT\r\n").await;
            said
        }
    }
}

/// The final line of an SMTP reply, which is the one whose code is followed by a space rather than
/// a hyphen. Every capability EHLO advertises arrives on a hyphenated line before it.
fn last_smtp_line(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 4 && bytes[..3].iter().all(u8::is_ascii_digit) && bytes[3] == b' '
}

async fn read_until(socket: &mut Stream, done: impl Fn(&str) -> bool) -> Option<String> {
    let mut seen: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.ok()?;
        if count == 0 {
            // The server closed on us. Whatever it managed to say first is still evidence, and a
            // greeting followed by a disconnect is what an IP-blocked client sees.
            let text = String::from_utf8_lossy(&seen).into_owned();
            return (!text.is_empty()).then_some(text);
        }
        seen.extend_from_slice(&buffer[..count]);
        // Decoded on every pass rather than at the end because the terminator is a line and lines
        // are text. Both protocols are ASCII on this path, so the borrow is free in practice.
        let text = String::from_utf8_lossy(&seen);
        if done(&text) || seen.len() >= MOST {
            return Some(text.into_owned());
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------------------------

/// A client of discovery's own, which exists for its User-Agent.
///
/// The central database sits behind Cloudflare, and a request that does not look like something a
/// person is driving is one challenge page away from being read as a configuration that failed to
/// parse. A browser string is not a trick here so much as an accurate description: this request is
/// made because somebody typed their address into a box and pressed return.
///
/// Redirects are followed but kept short. One hop is a hoster sending HTTP to HTTPS and is the
/// reason the plain variants are tried at all; a long chain is a captive portal or a login wall,
/// and neither of those is going to end in a clientConfig document.
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
        )
        .connect_timeout(HOSTED_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .referer(false)
        .build()
        .expect("could not build the discovery client")
});

/// The body of a 200, or nothing. Every other outcome is the same outcome here: a 404 from the
/// database for a domain it has never heard of and a connection refused by a host that does not
/// exist both mean this rung has no answer, and there is nothing to tell anybody about either.
async fn fetch(url: &str, timeout: Duration) -> Option<String> {
    let response = HTTP
        .get(url)
        .header("accept", "application/xml,text/xml,*/*")
        .timeout(timeout)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().await.ok()?;
    (!body.trim().is_empty()).then_some(body)
}

// ---------------------------------------------------------------------------------------------
// The clientConfig document
// ---------------------------------------------------------------------------------------------

/// Reads one document into a configuration, or decides it did not answer.
///
/// "Did not answer" covers more than a parse failure. A document listing only Exchange, or only
/// POP, or only mechanisms this app cannot perform is well-formed and complete and still leaves
/// nothing to connect with, and treating that as an answer would put a form in front of somebody
/// with the wrong host already typed into it.
fn parse(xml: &str, email: &str, source: &str) -> Option<MailConfig> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let root = doc.root_element();
    if root.tag_name().name() != "clientConfig" {
        return None;
    }
    let provider = children(root, "emailProvider").next()?;

    // Scoped to the provider's own children rather than searched for in the document, because
    // `<webMail><loginPageInfo>` carries a `<username>` of its own and gmail.com's document has
    // one. A descendant search would find the browser's login form.
    let imap = children(provider, "incomingServer")
        .filter(|server| server.attribute("type").is_some_and(|kind| eq(kind, "imap")))
        .find_map(|server| read_server(server, email))?;
    let smtp = children(provider, "outgoingServer")
        .filter(|server| server.attribute("type").is_none_or(|kind| eq(kind, "smtp")))
        .find_map(|server| read_server(server, email))?;

    // Bound before the struct literal because the iterator borrows the document, and a temporary
    // that lives to the end of the block would outlive it.
    let display_name = children(provider, "displayName")
        .find_map(|node| node.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);

    Some(MailConfig {
        imap,
        smtp,
        source: source.to_string(),
        display_name,
    })
}

fn children<'a>(
    node: roxmltree::Node<'a, 'a>,
    name: &'static str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}

fn text<'a>(node: roxmltree::Node<'a, 'a>, name: &'static str) -> Option<&'a str> {
    children(node, name)
        .find_map(|child| child.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn eq(found: &str, wanted: &str) -> bool {
    found.trim().eq_ignore_ascii_case(wanted)
}

/// One `<incomingServer>` or `<outgoingServer>`, or nothing when it is missing a part this app
/// cannot invent. A server with no port is not a server with a default port: the document is the
/// authority here and guessing at it would be the probe wearing the autoconfig rung's name.
fn read_server(node: roxmltree::Node<'_, '_>, email: &str) -> Option<ServerConfig> {
    let host = text(node, "hostname")?;
    let port: u16 = text(node, "port")?.parse().ok()?;
    let security = match text(node, "socketType")? {
        value if eq(value, "plain") => Security::Plain,
        value if eq(value, "STARTTLS") => Security::StartTls,
        // Real documents say SSL and mean TLS from the first byte. `TLS` is accepted alongside it
        // because a handful of hand-written files use the modern word for the same thing.
        value if eq(value, "SSL") || eq(value, "TLS") => Security::Tls,
        _ => return None,
    };
    let auth = children(node, "authentication")
        .filter_map(|child| child.text())
        .find_map(auth_kind)?;

    Some(ServerConfig {
        host: expand(host, email),
        port,
        security,
        auth,
        // An absent username means the address, which is what almost every server wants and what
        // the ones that publish nothing are assuming.
        username: expand(text(node, "username").unwrap_or("%EMAILADDRESS%"), email),
    })
}

/// The first mechanism family this app can perform, in the document's own order.
///
/// The order in the file is the provider's preference and it is worth honouring: gmail.com lists
/// OAuth2 before password-cleartext, and OAuth2 is the mechanism Google actually wants used. It is
/// returned because `AuthKind` can represent it, but nothing builds an OAuth IMAP session yet, so
/// an account discovered this way reaches the connect screen with a mechanism no code path serves.
/// That is the honest answer rather than a quiet downgrade to a password Google will refuse.
///
/// `GSSAPI` and `NTLM` are skipped rather than refused, because a document that lists one of them
/// first usually lists a password mechanism after it.
fn auth_kind(named: &str) -> Option<AuthKind> {
    let named = named.trim();
    // `plain` and `secure` are the deprecated spellings of the two password families and still
    // appear in documents nobody has touched in a decade.
    if eq(named, "password-cleartext")
        || eq(named, "plain")
        || eq(named, "password-encrypted")
        || eq(named, "secure")
    {
        return Some(AuthKind::Password);
    }
    eq(named, "OAuth2").then_some(AuthKind::OAuth2)
}

/// The placeholders, substituted here so that nothing downstream has to know the format has any.
fn expand(value: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    value
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The captured documents. Real files fetched with curl and committed unchanged, because the
    // shapes worth testing against are the ones providers actually publish: a loopback bridge, a
    // document that prefers OAuth, a document listing four incoming servers of two protocols.
    const PROTON: &str = include_str!("../../fixtures/autoconfig/proton.xml");
    const GMAIL: &str = include_str!("../../fixtures/autoconfig/gmail.xml");
    const POSTEO: &str = include_str!("../../fixtures/autoconfig/posteo.xml");
    const FASTMAIL: &str = include_str!("../../fixtures/autoconfig/fastmail.xml");

    struct Expected {
        name: &'static str,
        xml: &'static str,
        email: &'static str,
        display_name: Option<&'static str>,
        imap: (&'static str, u16, Security, AuthKind, &'static str),
        smtp: (&'static str, u16, Security, AuthKind, &'static str),
    }

    #[test]
    fn every_captured_document_reads_as_the_configuration_it_describes() {
        let table = [
            Expected {
                name: "proton",
                xml: PROTON,
                email: "someone@proton.me",
                display_name: Some("ProtonMail"),
                // The whole reason Proton works: Bridge on the loopback, upgraded by STARTTLS, on
                // ports nothing else in this table uses.
                imap: (
                    "127.0.0.1",
                    1143,
                    Security::StartTls,
                    AuthKind::Password,
                    "someone@proton.me",
                ),
                smtp: (
                    "127.0.0.1",
                    1025,
                    Security::StartTls,
                    AuthKind::Password,
                    "someone@proton.me",
                ),
            },
            Expected {
                name: "gmail",
                xml: GMAIL,
                email: "someone@gmail.com",
                display_name: Some("Google Mail"),
                // OAuth2 is listed first and is what comes back. The pop3 server between the two
                // that are wanted is skipped on its type attribute.
                imap: (
                    "imap.gmail.com",
                    993,
                    Security::Tls,
                    AuthKind::OAuth2,
                    "someone@gmail.com",
                ),
                smtp: (
                    "smtp.gmail.com",
                    465,
                    Security::Tls,
                    AuthKind::OAuth2,
                    "someone@gmail.com",
                ),
            },
            Expected {
                name: "posteo",
                xml: POSTEO,
                email: "someone@posteo.de",
                display_name: Some("Posteo"),
                // Two imap servers and two smtp servers, differing only in socketType. The first
                // of each wins, which is the implicit TLS one, which is the one to prefer.
                imap: (
                    "posteo.de",
                    993,
                    Security::Tls,
                    AuthKind::Password,
                    "someone@posteo.de",
                ),
                smtp: (
                    "posteo.de",
                    465,
                    Security::Tls,
                    AuthKind::Password,
                    "someone@posteo.de",
                ),
            },
            Expected {
                name: "fastmail",
                xml: FASTMAIL,
                email: "someone@fastmail.com",
                display_name: Some("Fastmail"),
                imap: (
                    "imap.fastmail.com",
                    993,
                    Security::Tls,
                    AuthKind::OAuth2,
                    "someone@fastmail.com",
                ),
                smtp: (
                    "smtp.fastmail.com",
                    465,
                    Security::Tls,
                    AuthKind::OAuth2,
                    "someone@fastmail.com",
                ),
            },
        ];

        for case in table {
            let found = parse(case.xml, case.email, "autoconfig")
                .unwrap_or_else(|| panic!("{} did not parse", case.name));
            assert_eq!(found.source, "autoconfig", "{}", case.name);
            assert_eq!(
                found.display_name.as_deref(),
                case.display_name,
                "{} display name",
                case.name
            );
            for (leg, found, wanted) in [
                ("imap", &found.imap, case.imap),
                ("smtp", &found.smtp, case.smtp),
            ] {
                assert_eq!(found.host, wanted.0, "{} {leg} host", case.name);
                assert_eq!(found.port, wanted.1, "{} {leg} port", case.name);
                assert_eq!(found.security, wanted.2, "{} {leg} security", case.name);
                assert_eq!(found.auth, wanted.3, "{} {leg} auth", case.name);
                assert_eq!(found.username, wanted.4, "{} {leg} username", case.name);
            }
        }
    }

    fn document(incoming: &str, outgoing: &str) -> String {
        format!(
            "<clientConfig version=\"1.1\"><emailProvider id=\"test\">\
               <displayName>Test</displayName>{incoming}{outgoing}\
             </emailProvider></clientConfig>"
        )
    }

    fn imap_server(body: &str) -> String {
        format!("<incomingServer type=\"imap\">{body}</incomingServer>")
    }

    fn smtp_server(body: &str) -> String {
        format!(
            "<outgoingServer type=\"smtp\"><hostname>smtp.test.example</hostname>\
             <port>587</port><socketType>STARTTLS</socketType>\
             <authentication>password-cleartext</authentication>{body}</outgoingServer>"
        )
    }

    const PLAIN_IMAP: &str = "<hostname>imap.test.example</hostname><port>993</port>\
         <socketType>SSL</socketType><authentication>password-cleartext</authentication>";

    #[test]
    fn all_three_username_placeholders_are_substituted() {
        let xml = document(
            &imap_server(
                "<hostname>imap.%EMAILDOMAIN%</hostname><port>993</port><socketType>SSL</socketType>\
                 <authentication>password-cleartext</authentication>\
                 <username>%EMAILLOCALPART%</username>",
            ),
            &smtp_server("<username>%EMAILADDRESS%</username>"),
        );
        let found = parse(&xml, "maya.hill@northgate.example", "autoconfig").expect("a config");
        assert_eq!(found.imap.username, "maya.hill");
        assert_eq!(found.smtp.username, "maya.hill@northgate.example");
        // The hostname carries them too, which is how a hoster serves one file for every domain.
        assert_eq!(found.imap.host, "imap.northgate.example");
    }

    #[test]
    fn an_absent_username_is_the_whole_address() {
        let xml = document(&imap_server(PLAIN_IMAP), &smtp_server(""));
        let found = parse(&xml, "maya@northgate.example", "autoconfig").expect("a config");
        assert_eq!(found.imap.username, "maya@northgate.example");
    }

    #[test]
    fn a_mechanism_we_cannot_perform_is_skipped_rather_than_taken() {
        let xml = document(
            &imap_server(
                "<hostname>imap.test.example</hostname><port>993</port><socketType>SSL</socketType>\
                 <authentication>GSSAPI</authentication>\
                 <authentication>NTLM</authentication>\
                 <authentication>password-cleartext</authentication>",
            ),
            &smtp_server(""),
        );
        let found = parse(&xml, "someone@test.example", "autoconfig").expect("a config");
        assert_eq!(found.imap.auth, AuthKind::Password);
    }

    #[test]
    fn a_document_offering_only_gssapi_does_not_answer() {
        let xml = document(
            &imap_server(
                "<hostname>imap.test.example</hostname><port>993</port><socketType>SSL</socketType>\
                 <authentication>GSSAPI</authentication>",
            ),
            &smtp_server(""),
        );
        assert!(parse(&xml, "someone@test.example", "autoconfig").is_none());
    }

    #[test]
    fn the_deprecated_spellings_still_mean_a_password() {
        for named in ["plain", "PLAIN", "secure", "password-encrypted", "Password-Cleartext"] {
            assert_eq!(auth_kind(named), Some(AuthKind::Password), "{named}");
        }
        assert_eq!(auth_kind("OAuth2"), Some(AuthKind::OAuth2));
        assert_eq!(auth_kind("GSSAPI"), None);
        assert_eq!(auth_kind("NTLM"), None);
    }

    #[test]
    fn a_pop_server_listed_first_does_not_win_over_the_imap_one_after_it() {
        let pop = "<incomingServer type=\"pop3\"><hostname>pop.test.example</hostname>\
             <port>995</port><socketType>SSL</socketType>\
             <authentication>password-cleartext</authentication></incomingServer>";
        let xml = document(
            &format!("{pop}{}", imap_server(PLAIN_IMAP)),
            &smtp_server(""),
        );
        let found = parse(&xml, "someone@test.example", "autoconfig").expect("a config");
        assert_eq!(found.imap.host, "imap.test.example");
        assert_eq!(found.imap.port, 993);
    }

    #[test]
    fn a_document_with_no_imap_server_at_all_does_not_answer() {
        let exchange = "<incomingServer type=\"exchange\"><hostname>owa.test.example</hostname>\
             <port>443</port><socketType>SSL</socketType>\
             <authentication>password-cleartext</authentication></incomingServer>";
        let xml = document(exchange, &smtp_server(""));
        assert!(parse(&xml, "someone@test.example", "autoconfig").is_none());
    }

    #[test]
    fn something_that_is_not_a_client_config_is_not_read_as_one() {
        // The failure this guards against is a catch-all host answering every path with its front
        // page, which is a 200 with a body and would otherwise reach the parser.
        assert!(parse("<html><body>Not found</body></html>", "a@b.example", "autoconfig").is_none());
        assert!(parse("", "a@b.example", "autoconfig").is_none());
        assert!(parse("{\"error\": true}", "a@b.example", "autoconfig").is_none());
    }

    #[test]
    fn the_socket_types_map_to_the_three_securities_and_nothing_else_does() {
        for (named, wanted) in [
            ("SSL", Security::Tls),
            ("ssl", Security::Tls),
            ("TLS", Security::Tls),
            ("STARTTLS", Security::StartTls),
            ("starttls", Security::StartTls),
            ("plain", Security::Plain),
            ("PLAIN", Security::Plain),
        ] {
            let body = format!(
                "<hostname>imap.test.example</hostname><port>143</port>\
                 <socketType>{named}</socketType>\
                 <authentication>password-cleartext</authentication>"
            );
            let xml = document(&imap_server(&body), &smtp_server(""));
            let found = parse(&xml, "a@test.example", "autoconfig").expect("a config");
            assert_eq!(found.imap.security, wanted, "{named}");
        }

        let body = "<hostname>imap.test.example</hostname><port>143</port>\
             <socketType>whatever</socketType>\
             <authentication>password-cleartext</authentication>";
        let xml = document(&imap_server(body), &smtp_server(""));
        assert!(parse(&xml, "a@test.example", "autoconfig").is_none());
    }

    // -----------------------------------------------------------------------------------------
    // The MX rung's arithmetic
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_base_domain_is_the_registrable_one() {
        assert_eq!(base_domain("aspmx.l.google.com").as_deref(), Some("google.com"));
        assert_eq!(
            base_domain("in1-smtp.messagingengine.com").as_deref(),
            Some("messagingengine.com")
        );
        assert_eq!(base_domain("example.com").as_deref(), Some("example.com"));
        assert_eq!(base_domain("mx.example.com.").as_deref(), Some("example.com"));
        assert_eq!(base_domain("MX.EXAMPLE.COM").as_deref(), Some("example.com"));
        // A hostname with nothing to reduce is not a domain to ask about.
        assert_eq!(base_domain("localhost"), None);
        assert_eq!(base_domain(""), None);
    }

    #[test]
    fn a_two_level_suffix_takes_three_labels_rather_than_two() {
        assert_eq!(
            base_domain("mx.mail.northgate.co.uk").as_deref(),
            Some("northgate.co.uk")
        );
        assert_eq!(base_domain("northgate.co.uk").as_deref(), Some("northgate.co.uk"));
        // `co.uk` on its own has nothing registrable under it in the name, and returning `co.uk`
        // here would send the rung off to ask Nominet about somebody's mail.
        assert_eq!(base_domain("co.uk").as_deref(), Some("co.uk"));
        assert_eq!(base_domain("mx.northgate.com.au").as_deref(), Some("northgate.com.au"));
        assert_eq!(base_domain("mx.northgate.co.jp").as_deref(), Some("northgate.co.jp"));
        // The counterpart: a two-label name that merely looks like one of the suffixes.
        assert_eq!(base_domain("mx.company.uk").as_deref(), Some("company.uk"));
    }

    #[test]
    fn an_exchange_yields_the_operator_and_the_label_above_it() {
        assert_eq!(mx_domains("aspmx.l.google.com"), ["google.com", "l.google.com"]);
        assert_eq!(
            mx_domains("in1-smtp.messagingengine.com"),
            ["messagingengine.com", "in1-smtp.messagingengine.com"]
        );
        assert_eq!(
            mx_domains("mx.northgate.co.uk"),
            ["northgate.co.uk", "mx.northgate.co.uk"]
        );
        // Nothing above the base means one candidate, not a duplicate.
        assert_eq!(mx_domains("messagingengine.com"), ["messagingengine.com"]);
        assert!(mx_domains("localhost").is_empty());
    }

    // -----------------------------------------------------------------------------------------
    // The probe's arithmetic
    // -----------------------------------------------------------------------------------------

    fn outcomes(marks: &str) -> Vec<(char, Outcome)> {
        // `.` failed, `?` still running, `!` answered. Written as a string so a case reads as the
        // shape of the race rather than as nine lines of enum.
        marks
            .chars()
            .zip('a'..)
            .map(|(mark, name)| {
                (
                    name,
                    match mark {
                        '.' => Outcome::Failed,
                        '!' => Outcome::Answered,
                        _ => Outcome::Pending,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn the_first_answer_wins_only_once_everything_better_has_definitely_failed() {
        // Nothing has reported yet.
        assert_eq!(first_answer(&outcomes("?????")), None);
        // The best one answered, and there is nothing above it to wait for.
        assert_eq!(first_answer(&outcomes("!????")).copied(), Some('a'));
        // A worse one answered first. The better one is still open, so the race is not over: this
        // is the case that stops `mail.` on 143 beating `imap.` on 993 by being quicker to refuse.
        assert_eq!(first_answer(&outcomes("??!??")), None);
        // Now the better ones have failed, so the worse one is the winner.
        assert_eq!(first_answer(&outcomes("..!??")).copied(), Some('c'));
        // Everything failed.
        assert_eq!(first_answer(&outcomes(".....")), None);
        // Two answered and the better one takes it.
        assert_eq!(first_answer(&outcomes(".!.!.")).copied(), Some('b'));
        assert_eq!(first_answer::<char>(&[]), None);
    }

    #[test]
    fn the_probe_prefers_tls_then_starttls_then_plain_and_the_named_host_before_the_domain() {
        let hosts: Vec<String> = INCOMING
            .iter()
            .map(|candidate| {
                format!(
                    "{}:{} {:?}",
                    candidate.host("northgate.example"),
                    candidate.port,
                    candidate.security
                )
            })
            .collect();
        assert_eq!(
            hosts,
            [
                "imap.northgate.example:993 Tls",
                "mail.northgate.example:993 Tls",
                "northgate.example:993 Tls",
                "imap.northgate.example:143 StartTls",
                "mail.northgate.example:143 StartTls",
                "northgate.example:143 StartTls",
                "imap.northgate.example:143 Plain",
                "mail.northgate.example:143 Plain",
                "northgate.example:143 Plain",
            ]
        );
        // No POP anywhere in it, which is a promise rather than an accident.
        assert!(INCOMING.iter().all(|c| c.port != 110 && c.port != 995));
        assert_eq!(
            OUTGOING.first().map(|c| (c.prefix, c.port, c.security)),
            Some(("smtp", 465, Security::Tls))
        );
        assert!(OUTGOING.iter().all(|c| c.port == 465 || c.port == 587));
    }

    #[test]
    fn the_last_line_of_an_smtp_reply_is_the_one_without_the_hyphen() {
        assert!(last_smtp_line("250 mail.example.com"));
        assert!(!last_smtp_line("250-STARTTLS"));
        assert!(last_smtp_line("530 Access denied"));
        assert!(!last_smtp_line("STARTTLS"));
        assert!(!last_smtp_line(""));
        assert!(!last_smtp_line("250"));
    }

    // -----------------------------------------------------------------------------------------
    // The address, and the URLs it becomes
    // -----------------------------------------------------------------------------------------

    #[test]
    fn an_address_that_could_choose_its_own_host_is_refused() {
        assert_eq!(domain_of("maya@Northgate.Example").as_deref(), Some("northgate.example"));
        assert_eq!(domain_of("maya@northgate.example.").as_deref(), Some("northgate.example"));
        // A plus address is ordinary and the domain is still the domain.
        assert_eq!(domain_of("maya+lists@northgate.example").as_deref(), Some("northgate.example"));
        assert_eq!(domain_of("maya@northgate.example/evil.test"), None);
        assert_eq!(domain_of("maya@northgate.example?x=1"), None);
        assert_eq!(domain_of("maya@nor thgate.example"), None);
        assert_eq!(domain_of("maya@"), None);
        assert_eq!(domain_of("@northgate.example"), None);
        assert_eq!(domain_of("maya"), None);
    }

    #[test]
    fn the_hosted_urls_are_the_four_thunderbird_tries_in_the_order_it_tries_them() {
        let urls = hosted_urls("northgate.example", "maya+lists@northgate.example", Reach::Anywhere);
        assert_eq!(
            urls,
            [
                "https://autoconfig.northgate.example/mail/config-v1.1.xml?emailaddress=maya%2Blists%40northgate.example",
                "https://northgate.example/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=maya%2Blists%40northgate.example",
                "http://autoconfig.northgate.example/mail/config-v1.1.xml?emailaddress=maya%2Blists%40northgate.example",
                "http://northgate.example/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress=maya%2Blists%40northgate.example",
            ]
        );
    }

    #[test]
    fn a_domain_reached_by_inference_is_asked_over_https_at_one_address_only() {
        let urls = hosted_urls("google.com", "maya@northgate.example", Reach::HttpsOnly);
        assert_eq!(
            urls,
            ["https://autoconfig.google.com/mail/config-v1.1.xml?emailaddress=maya%40northgate.example"]
        );
    }

    // -----------------------------------------------------------------------------------------
    // The rungs against the real internet
    //
    // Ignored, because `cargo test` has to pass on a train. Run them with:
    //   cargo test --lib imap::discover::tests -- --ignored --nocapture
    // -----------------------------------------------------------------------------------------

    #[test]
    #[ignore = "hits the network"]
    fn proton_really_answers_with_the_bridge_on_the_loopback() {
        let found = tauri::async_runtime::block_on(discover("someone@proton.me"))
            .expect("proton publishes a configuration");
        assert_eq!(found.source, "autoconfig");
        assert_eq!(found.imap.host, "127.0.0.1");
        assert_eq!(found.imap.port, 1143);
        assert_eq!(found.imap.security, Security::StartTls);
        assert_eq!(found.smtp.port, 1025);
        assert_eq!(found.imap.username, "someone@proton.me");
    }

    #[test]
    #[ignore = "hits the network"]
    fn gmail_really_answers_from_the_central_database() {
        let found = tauri::async_runtime::block_on(discover("someone@gmail.com"))
            .expect("gmail is in the database");
        assert_eq!(found.imap.host, "imap.gmail.com");
        assert_eq!(found.imap.port, 993);
        assert_eq!(found.imap.security, Security::Tls);
        assert_eq!(found.smtp.host, "smtp.gmail.com");
    }

    #[test]
    #[ignore = "hits the network"]
    fn a_domain_nobody_publishes_anything_about_falls_through_to_the_probe_or_to_nothing() {
        // `example.com` has no autoconfig, no database entry and an MX that refuses everything, so
        // this is the shape of the walk to the manual sheet.
        let found = tauri::async_runtime::block_on(discover("someone@example.com"));
        assert!(
            found.as_ref().is_none_or(|found| found.source == "probe"),
            "unexpected rung answered: {found:?}"
        );
    }

    #[test]
    #[ignore = "hits the network"]
    fn the_probe_finds_a_real_pair_of_servers_without_being_told_about_them() {
        // Run against Google on purpose, even though the rungs above would have answered first: it
        // is the one domain where the right answer is already known, so a probe that finds anything
        // else has found the wrong thing rather than merely found nothing.
        let found = tauri::async_runtime::block_on(probe(
            "gmail.com".to_string(),
            "someone@gmail.com".to_string(),
        ))
        .expect("the probe finds Google");
        assert_eq!(found.source, "probe");
        assert_eq!(found.imap.host, "imap.gmail.com");
        assert_eq!(found.imap.port, 993);
        assert_eq!(found.imap.security, Security::Tls);
        assert_eq!(found.smtp.host, "smtp.gmail.com");
        assert_eq!(found.smtp.port, 465);
        // A guess is always a password guess, whatever the provider would prefer.
        assert_eq!(found.imap.auth, AuthKind::Password);
    }

    #[test]
    #[ignore = "hits the network"]
    fn a_probe_answers_only_when_the_socket_really_speaks_the_protocol() {
        let block = tauri::async_runtime::block_on;
        assert!(block(speaks("imap.gmail.com", 993, Security::Tls, Leg::Imap)));
        assert!(block(speaks("smtp.gmail.com", 465, Security::Tls, Leg::Smtp)));
        // Posteo is the STARTTLS case: 143 in the clear, advertising the upgrade in its capability
        // list, which is the only thing that makes a `start-tls` candidate a success.
        assert!(block(speaks("posteo.de", 143, Security::StartTls, Leg::Imap)));
        assert!(block(speaks("posteo.de", 587, Security::StartTls, Leg::Smtp)));
        // The same port asked the wrong question. Implicit TLS on 143 never gets a greeting out.
        assert!(!block(speaks("posteo.de", 143, Security::Tls, Leg::Imap)));
        // A host that has no mail server behind it at all.
        assert!(!block(speaks("example.com", 993, Security::Tls, Leg::Imap)));
    }

    #[test]
    #[ignore = "hits the network, and reads somebody else's DNS"]
    fn the_mx_rung_recognises_a_vanity_domain_on_google_workspace() {
        // Tailscale publishes no autoconfig, is not in the database, and has its mail delivered to
        // `aspmx.l.google.com`. That is the whole shape of the rung in one domain. It is somebody
        // else's DNS, so the day they move providers this test is telling the truth about the
        // internet rather than lying about the code.
        let exchange =
            tauri::async_runtime::block_on(lowest_mx("tailscale.com")).expect("an MX record");
        assert!(
            mx_domains(&exchange).contains(&"google.com".to_string()),
            "{exchange} did not reduce to google.com"
        );

        let found = tauri::async_runtime::block_on(discover("someone@tailscale.com"))
            .expect("the MX rung answers");
        assert_eq!(found.source, "mx");
        assert_eq!(found.imap.host, "imap.gmail.com");
        assert_eq!(found.imap.port, 993);
        // Substituted against the address that was typed, not against the operator's domain.
        assert_eq!(found.imap.username, "someone@tailscale.com");
    }
}
