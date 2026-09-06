// What a server calls its mailboxes, mapped onto the roles the app needs.
//
// OWNED BY THE IMAP PROVIDER PACKAGE.
//
// Two passes, and the order between them is the whole point. A server that says `\Sent` about a
// mailbox is telling us something it knows; a mailbox called "Sent" is us guessing, and a guess
// must never overrule an answer. So every flag is read first, and the name table only fills what
// the flags left empty.
//
// The name table is written here rather than borrowed. Mailspring's is GPL, and it is also wrong
// in at least two places: it maps the Spanish "borradores", which means drafts, onto trash. A
// table with a bug like that in it silently deletes mail, so this one is deliberately small and
// English, plus the `[Gmail]/` paths, and everything else falls through to `\Noselect`-safe
// nothing. A missing role is recoverable; a wrong one is not.
//
// There is no NAMESPACE command here on purpose. `imap-proto` has no NAMESPACE response, so
// sending one would put bytes on the wire that the parser cannot read and take the connection
// down with it. The one thing NAMESPACE would have been used for, an `INBOX.`-prefixed personal
// namespace, is visible in the listing itself: if the server's own Sent and Drafts live under
// INBOX, so does anything this app creates.

use async_imap::imap_proto::{MailboxDatum, NameAttribute, Response};

use super::session::{self, Session};
use super::tls::Refused;

/// The six roles anything above this cares about, each holding the server's own path for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roles {
    pub inbox: Option<String>,
    pub sent: Option<String>,
    pub drafts: Option<String>,
    pub trash: Option<String>,
    pub spam: Option<String>,
    pub archive: Option<String>,
    /// Gmail's All Mail, and anything else that advertises `\All`.
    pub all: Option<String>,
}

/// Which slot a mailbox fills. Only ever used to name one of the fields above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Spam,
    Archive,
    All,
}

impl Roles {
    fn slot(&mut self, role: Role) -> &mut Option<String> {
        match role {
            Role::Inbox => &mut self.inbox,
            Role::Sent => &mut self.sent,
            Role::Drafts => &mut self.drafts,
            Role::Trash => &mut self.trash,
            Role::Spam => &mut self.spam,
            Role::Archive => &mut self.archive,
            Role::All => &mut self.all,
        }
    }

    /// The role a path has, if any. Linear over seven fields, which is cheaper than keeping a
    /// second map in step with the first.
    pub fn role_of(&self, path: &str) -> Option<Role> {
        for (role, held) in [
            (Role::Inbox, &self.inbox),
            (Role::Sent, &self.sent),
            (Role::Drafts, &self.drafts),
            (Role::Trash, &self.trash),
            (Role::Spam, &self.spam),
            (Role::Archive, &self.archive),
            (Role::All, &self.all),
        ] {
            if held.as_deref() == Some(path) {
                return Some(role);
            }
        }
        None
    }
}

/// One mailbox exactly as the server listed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listed {
    pub path: String,
    /// The server's hierarchy delimiter, empty when the names are flat.
    pub delimiter: String,
    /// The name attributes, as written: `\Noselect`, `\Sent`, `\HasChildren` and so on.
    pub flags: Vec<String>,
}

impl Listed {
    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|held| held.eq_ignore_ascii_case(flag))
    }

    /// A mailbox that can be SELECTed. The two spellings are the same refusal: `\NonExistent`
    /// arrives from RFC 5258 servers and implies `\Noselect`.
    pub fn selectable(&self) -> bool {
        !self.has("\\Noselect") && !self.has("\\NonExistent")
    }
}

/// Every mailbox the account can see.
pub async fn list(session: &mut Session) -> Result<Vec<Listed>, Refused> {
    listing(session, "*").await
}

async fn listing(session: &mut Session, pattern: &str) -> Result<Vec<Listed>, Refused> {
    let mut listed: Vec<Listed> = Vec::new();
    session
        .command(
            &format!("LIST \"\" {}", session::quoted(pattern)),
            |response| {
                if let Response::MailboxData(MailboxDatum::List {
                    name_attributes,
                    delimiter,
                    name,
                }) = response
                {
                    listed.push(Listed {
                        path: name.to_string(),
                        delimiter: delimiter.as_deref().unwrap_or("").to_string(),
                        flags: name_attributes.iter().map(flag_name).collect(),
                    });
                }
            },
        )
        .await
        .map_err(|e| Refused::Other(e.to_string()))?;
    Ok(listed)
}

fn flag_name(attribute: &NameAttribute<'_>) -> String {
    match attribute {
        NameAttribute::NoInferiors => "\\Noinferiors".to_string(),
        NameAttribute::NoSelect => "\\Noselect".to_string(),
        NameAttribute::Marked => "\\Marked".to_string(),
        NameAttribute::Unmarked => "\\Unmarked".to_string(),
        NameAttribute::All => "\\All".to_string(),
        NameAttribute::Archive => "\\Archive".to_string(),
        NameAttribute::Drafts => "\\Drafts".to_string(),
        NameAttribute::Flagged => "\\Flagged".to_string(),
        NameAttribute::Junk => "\\Junk".to_string(),
        NameAttribute::Sent => "\\Sent".to_string(),
        NameAttribute::Trash => "\\Trash".to_string(),
        // `\Inbox`, `\Spam` and `\Important` all arrive here: they are XLIST rather than RFC 6154,
        // and the parser has no case for them.
        NameAttribute::Extension(name) => name.to_string(),
        // The enum is `non_exhaustive`, and an attribute this build has never heard of is exactly
        // the kind of thing the name table is the fallback for.
        _ => String::new(),
    }
}

/// Reads the mailbox list and assigns roles: SPECIAL-USE and XLIST flags first, then names.
pub async fn roles(session: &mut Session) -> Result<Roles, Refused> {
    Ok(assign(&list(session).await?))
}

/// The assignment itself, as a function of the listing and nothing else.
pub fn assign(listed: &[Listed]) -> Roles {
    let mut roles = Roles::default();

    // What the server says about itself. `\Junk` is RFC 6154 and `\Spam` is what several XLIST
    // servers send for the same mailbox, so both fill the same slot.
    for entry in listed.iter().filter(|entry| entry.selectable()) {
        for flag in &entry.flags {
            let role = match flag.to_ascii_lowercase().as_str() {
                "\\inbox" => Role::Inbox,
                "\\sent" => Role::Sent,
                "\\drafts" => Role::Drafts,
                "\\trash" => Role::Trash,
                "\\junk" | "\\spam" => Role::Spam,
                "\\archive" => Role::Archive,
                "\\all" | "\\allmail" => Role::All,
                _ => continue,
            };
            let slot = roles.slot(role);
            if slot.is_none() {
                *slot = Some(entry.path.clone());
            }
        }
    }

    // Then the guesses, and only where nothing was said.
    for entry in listed.iter().filter(|entry| entry.selectable()) {
        let Some(role) = named(&entry.path, &entry.delimiter) else {
            continue;
        };
        let slot = roles.slot(role);
        if slot.is_none() {
            *slot = Some(entry.path.clone());
        }
    }

    roles
}

/// The whole path first, then the last segment of it, so both `[Gmail]/Sent Mail` and
/// `INBOX.Sent` land on Sent without the table having to know either shape.
fn named(path: &str, delimiter: &str) -> Option<Role> {
    let lowered = path.to_ascii_lowercase();
    if let Some(role) = from_name(&lowered) {
        return Some(role);
    }
    from_name(leaf(&lowered, delimiter))
}

fn leaf<'a>(path: &'a str, delimiter: &str) -> &'a str {
    let separators: Vec<char> = if delimiter.is_empty() {
        vec!['/', '.']
    } else {
        delimiter.chars().collect()
    };
    path.rsplit(separators.as_slice()).next().unwrap_or(path)
}

/// The table. Deliberately English and deliberately short: a name this does not recognise costs a
/// role that `ensure_archive` can make or that the app can do without, and a name it recognises
/// wrongly costs somebody their mail.
fn from_name(name: &str) -> Option<Role> {
    Some(match name {
        "inbox" => Role::Inbox,
        // Not "outbox": that is mail on its way out, and treating it as the sent copy would file
        // every queued message as already gone.
        "sent" | "sent items" | "sent mail" | "sent messages" | "[gmail]/sent mail"
        | "[google mail]/sent mail" => Role::Sent,
        "drafts" | "draft" | "[gmail]/drafts" | "[google mail]/drafts" => Role::Drafts,
        "trash" | "bin" | "deleted" | "deleted items" | "deleted messages" | "[gmail]/trash"
        | "[gmail]/bin" | "[google mail]/trash" | "[google mail]/bin" => Role::Trash,
        "spam" | "junk" | "junk email" | "junk e-mail" | "bulk mail" | "[gmail]/spam"
        | "[google mail]/spam" => Role::Spam,
        "archive" | "archives" | "[gmail]/archive" => Role::Archive,
        // Only the unambiguous spellings. Guessing `\All` about a folder somebody named "All"
        // would take it out of the sync entirely, which is a far worse mistake than missing it.
        "all mail" | "[gmail]/all mail" | "[google mail]/all mail" => Role::All,
        _ => return None,
    })
}

/// Creates an Archive mailbox when the server has none, because every keybinding and every place
/// in this app means the same thing on every account or the app is two apps.
///
/// A failure to create is not a failure to sync. The caller degrades: with no archive, the archive
/// keystroke has nowhere to move mail to and says so, and the rest of the mailbox is unaffected.
pub async fn ensure_archive(session: &mut Session, roles: &mut Roles) -> Result<(), Refused> {
    if roles.archive.is_some() {
        return Ok(());
    }

    // One cheap top level LIST, only for the delimiter. `LIST "" "%"` is a handful of lines even
    // on an account with thousands of folders under them.
    let delimiter = listing(session, "%")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.delimiter)
        .find(|delimiter| !delimiter.is_empty())
        .unwrap_or_default();

    let path = archive_path(&delimiter, roles);
    if session
        .run(&format!("CREATE {}", session::quoted(&path)))
        .await
        .is_ok()
    {
        // An unsubscribed mailbox is invisible in other clients, which is a confusing thing to do
        // to somebody's account on their behalf.
        let _ = session
            .run(&format!("SUBSCRIBE {}", session::quoted(&path)))
            .await;
        roles.archive = Some(path);
        return Ok(());
    }

    // A refusal is most often "that already exists", which is the outcome we wanted. Asking for
    // its status is the cheapest way to tell that apart from a server that will not have one.
    if session
        .run(&format!("STATUS {} (UIDVALIDITY)", session::quoted(&path)))
        .await
        .is_ok()
    {
        roles.archive = Some(path);
    }
    Ok(())
}

/// Where an Archive would go: the top level, unless the account's own mailboxes live under INBOX,
/// in which case so does this one.
pub fn archive_path(delimiter: &str, roles: &Roles) -> String {
    let prefix = [&roles.sent, &roles.drafts, &roles.trash, &roles.spam]
        .into_iter()
        .flatten()
        .find_map(|path| inbox_prefix(path, delimiter))
        .unwrap_or_default();
    format!("{prefix}Archive")
}

fn inbox_prefix(path: &str, delimiter: &str) -> Option<String> {
    if delimiter.is_empty() {
        return None;
    }
    let wanted = format!("INBOX{delimiter}");
    path.to_ascii_uppercase()
        .starts_with(&wanted)
        .then(|| path[..wanted.len()].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mailbox(path: &str, flags: &[&str]) -> Listed {
        Listed {
            path: path.to_string(),
            delimiter: "/".to_string(),
            flags: flags.iter().map(|flag| flag.to_string()).collect(),
        }
    }

    fn dotted(path: &str, flags: &[&str]) -> Listed {
        Listed {
            delimiter: ".".to_string(),
            ..mailbox(path, flags)
        }
    }

    #[test]
    fn a_server_that_names_its_own_mailboxes_is_believed() {
        let roles = assign(&[
            mailbox("INBOX", &["\\HasNoChildren"]),
            mailbox("Sent", &["\\Sent", "\\HasNoChildren"]),
            mailbox("Drafts", &["\\Drafts"]),
            mailbox("Trash", &["\\Trash"]),
            mailbox("Junk", &["\\Junk"]),
            mailbox("Archive", &["\\Archive"]),
        ]);
        assert_eq!(roles.inbox.as_deref(), Some("INBOX"));
        assert_eq!(roles.sent.as_deref(), Some("Sent"));
        assert_eq!(roles.drafts.as_deref(), Some("Drafts"));
        assert_eq!(roles.trash.as_deref(), Some("Trash"));
        assert_eq!(roles.spam.as_deref(), Some("Junk"));
        assert_eq!(roles.archive.as_deref(), Some("Archive"));
        assert_eq!(roles.all, None);
    }

    /// The precedence that matters: a flag on one mailbox beats a matching name on another, in
    /// either listing order.
    #[test]
    fn a_flag_beats_a_name_whichever_came_first_in_the_listing() {
        let flag_first = assign(&[
            mailbox("Archivio", &["\\Sent"]),
            mailbox("Sent", &["\\HasNoChildren"]),
        ]);
        assert_eq!(flag_first.sent.as_deref(), Some("Archivio"));

        let name_first = assign(&[
            mailbox("Sent", &["\\HasNoChildren"]),
            mailbox("Archivio", &["\\Sent"]),
        ]);
        assert_eq!(name_first.sent.as_deref(), Some("Archivio"));
    }

    #[test]
    fn junk_and_spam_are_the_same_slot_because_servers_disagree_about_which_one_to_send() {
        let junk = assign(&[mailbox("Junk", &["\\Junk"])]);
        let spam = assign(&[mailbox("Spam", &["\\Spam"])]);
        assert_eq!(junk.spam.as_deref(), Some("Junk"));
        assert_eq!(spam.spam.as_deref(), Some("Spam"));
    }

    #[test]
    fn gmails_bracketed_paths_are_read_by_name_when_it_sends_no_flags() {
        let roles = assign(&[
            mailbox("INBOX", &[]),
            mailbox("[Gmail]", &["\\Noselect", "\\HasChildren"]),
            mailbox("[Gmail]/All Mail", &[]),
            mailbox("[Gmail]/Sent Mail", &[]),
            mailbox("[Gmail]/Drafts", &[]),
            mailbox("[Gmail]/Trash", &[]),
            mailbox("[Gmail]/Spam", &[]),
        ]);
        assert_eq!(roles.sent.as_deref(), Some("[Gmail]/Sent Mail"));
        assert_eq!(roles.drafts.as_deref(), Some("[Gmail]/Drafts"));
        assert_eq!(roles.trash.as_deref(), Some("[Gmail]/Trash"));
        assert_eq!(roles.spam.as_deref(), Some("[Gmail]/Spam"));
        assert_eq!(roles.all.as_deref(), Some("[Gmail]/All Mail"));
        assert_eq!(roles.archive, None);
    }

    #[test]
    fn a_noselect_container_is_never_given_a_role() {
        let roles = assign(&[
            mailbox("Archive", &["\\Noselect", "\\HasChildren"]),
            mailbox("Archive/2024", &[]),
        ]);
        assert_eq!(roles.archive, None);
    }

    #[test]
    fn a_dotted_namespace_matches_on_the_last_segment() {
        let roles = assign(&[
            dotted("INBOX", &[]),
            dotted("INBOX.Sent", &[]),
            dotted("INBOX.Drafts", &[]),
            dotted("INBOX.Trash", &[]),
        ]);
        assert_eq!(roles.inbox.as_deref(), Some("INBOX"));
        assert_eq!(roles.sent.as_deref(), Some("INBOX.Sent"));
        assert_eq!(roles.drafts.as_deref(), Some("INBOX.Drafts"));
        assert_eq!(roles.trash.as_deref(), Some("INBOX.Trash"));
    }

    /// A folder somebody called "All" is theirs, not the server's virtual everything, and reading
    /// it as `\All` would drop it out of the sync without a word.
    #[test]
    fn only_the_unambiguous_spellings_are_read_as_all_mail() {
        assert_eq!(assign(&[mailbox("All", &[])]).all, None);
        assert_eq!(
            assign(&[mailbox("All Mail", &[])]).all.as_deref(),
            Some("All Mail")
        );
    }

    #[test]
    fn a_server_with_no_archive_at_all_leaves_the_slot_empty_for_ensure_archive() {
        let roles = assign(&[
            mailbox("INBOX", &[]),
            mailbox("Sent", &["\\Sent"]),
            mailbox("Trash", &["\\Trash"]),
            mailbox("Work", &[]),
            mailbox("Work/Invoices", &[]),
        ]);
        assert_eq!(roles.archive, None);
        assert_eq!(archive_path("/", &roles), "Archive");
    }

    #[test]
    fn an_archive_goes_under_inbox_when_the_accounts_own_mailboxes_are_there() {
        let roles = assign(&[
            dotted("INBOX", &[]),
            dotted("INBOX.Sent", &[]),
            dotted("INBOX.Trash", &[]),
        ]);
        assert_eq!(archive_path(".", &roles), "INBOX.Archive");
        // A flat server has no prefix to inherit, whatever its other mailboxes look like.
        assert_eq!(archive_path("", &roles), "Archive");
    }

    #[test]
    fn a_role_can_be_read_back_from_a_path() {
        let roles = assign(&[mailbox("INBOX", &[]), mailbox("Sent", &["\\Sent"])]);
        assert_eq!(roles.role_of("Sent"), Some(Role::Sent));
        assert_eq!(roles.role_of("INBOX"), Some(Role::Inbox));
        assert_eq!(roles.role_of("Work"), None);
    }
}
