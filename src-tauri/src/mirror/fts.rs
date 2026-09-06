// The search index and the query language.
//
// Parsing a query string is a pure function of the string, so it is tested on its own and the SQL
// it produces is assembled somewhere else. What comes out is an FTS5 match expression plus a set
// of predicates, because half of what the operators ask about (a date, a label, an attachment) is
// a column and not a word.

use rusqlite::{params, Connection, OptionalExtension};

use crate::dto::Person;

/// A parsed query. Every field is additive: two `from:` terms mean both, which is what a person
/// typing them expects even though it is the less useful reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub filename: Vec<String>,
    pub label: Vec<String>,
    /// `in:`, which names a place rather than a folder.
    pub place: Option<String>,
    pub has_attachment: bool,
    pub before_ms: Option<i64>,
    pub after_ms: Option<i64>,
}

impl Query {
    /// The FTS5 expression, or None when nothing in the query is a word. A query of nothing but
    /// `has:attachment` is answered by the predicates alone and must not reach the index.
    pub fn match_expr(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        for word in &self.words {
            parts.push(quoted(word));
        }
        for (column, terms) in [
            ("sender", &self.from),
            ("recipients", &self.to),
            ("subject", &self.subject),
            ("filenames", &self.filename),
        ] {
            for term in terms {
                parts.push(format!("{column} : {}", quoted(term)));
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// True when the query asks about a moment the device may not hold, which is the one case
    /// where a local answer would be confidently wrong rather than merely short.
    pub fn reaches_before(&self, window_start: Option<i64>) -> bool {
        match (self.before_ms, self.after_ms, window_start) {
            (_, _, None) => false,
            (Some(before), _, Some(start)) => before <= start,
            (_, Some(after), Some(start)) => after < start,
            _ => false,
        }
    }
}

/// FTS5 string literals are double quoted, and a double quote inside one is written twice. Every
/// term goes through here, so a query with a stray quote in it is a search and not a syntax error.
fn quoted(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Splits on whitespace, keeping a double quoted run together so `subject:"end of quarter"` is one
/// term. The quotes are removed; what is inside them is the term.
fn tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A bare date, as `2026-09-03` or `2026/09/03`, at midnight UTC. Anything else is not a date and
/// the operator carrying it is dropped rather than guessed at.
pub fn date_ms(value: &str) -> Option<i64> {
    let cleaned = value.replace('/', "-");
    chrono::NaiveDate::parse_from_str(&cleaned, "%Y-%m-%d")
        .ok()
        .map(|date| date.and_hms_opt(0, 0, 0).expect("midnight").and_utc().timestamp_millis())
}

pub fn parse(input: &str) -> Query {
    let mut query = Query::default();
    for token in tokens(input) {
        let Some((operator, value)) = token.split_once(':') else {
            query.words.push(token);
            continue;
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            query.words.push(token);
            continue;
        }
        match operator.to_ascii_lowercase().as_str() {
            "from" => query.from.push(value.to_lowercase()),
            "to" => query.to.push(value.to_lowercase()),
            "subject" => query.subject.push(value),
            "filename" => query.filename.push(value),
            "label" => query.label.push(value),
            "in" => query.place = Some(value.to_lowercase()),
            "has" if value.eq_ignore_ascii_case("attachment") => query.has_attachment = true,
            "before" => query.before_ms = date_ms(&value),
            "after" => query.after_ms = date_ms(&value),
            // An unknown operator is a colon in someone's search text, not a mistake to report.
            _ => query.words.push(token),
        }
    }
    query
}

// ---------------------------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------------------------

fn people(json: &str) -> String {
    serde_json::from_str::<Vec<Person>>(json)
        .unwrap_or_default()
        .into_iter()
        .map(|person| match person.name {
            Some(name) => format!("{name} {}", person.address),
            None => person.address,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Indexes one message. Called when a body arrives, and again when it is rendered a second time,
/// so a message whose body would not parse is still findable by its headers.
pub fn index(conn: &Connection, message_id: &str) -> Result<(), String> {
    let row: Option<(String, String, Option<String>, String, String, String)> = conn
        .query_row(
            "SELECT thread_key, subject, from_name, from_address, to_json, cc_json
             FROM messages WHERE id = ?1",
            [message_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((thread_key, subject, from_name, from_address, to_json, cc_json)) = row else {
        return Ok(());
    };

    let body: Option<String> = conn
        .query_row(
            "SELECT COALESCE(text, '') FROM bodies WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT filename FROM attachments WHERE message_id = ?1")
        .map_err(|e| e.to_string())?;
    let filenames: Vec<String> = stmt
        .query_map([message_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    remove(conn, message_id)?;
    conn.execute(
        "INSERT INTO search (message_id, thread_key, subject, sender, recipients, body, filenames)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            message_id,
            thread_key,
            subject,
            match from_name {
                Some(name) => format!("{name} {from_address}"),
                None => from_address,
            },
            format!("{} {}", people(&to_json), people(&cc_json)).trim(),
            body.unwrap_or_default(),
            filenames.join(" "),
        ],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn remove(conn: &Connection, message_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM search WHERE message_id = ?1", [message_id])
        .map(|_| ())
        .map_err(|e| e.to_string())
}
