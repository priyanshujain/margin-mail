// The People API, for the first run screening seed and for autocomplete's second pass.
//
// Two lists, two scopes, and they are not the same set. `connections` is the address book the user
// curated, which is small and often stale. `otherContacts` is the auto-collected "people you have
// emailed" set that Gmail's own autocomplete runs on, which is large and is the one that matters.
// Both are asked for once per install: the People API has its own quota with an extra charge on the
// first page of a full sync, and a client that re-lists on every launch gets 429ed for it.
//
// This is not on the `Provider` trait. The trait's `contacts` is what the sync engine calls, and it
// calls this; an IMAP provider answers it from the mirror instead, which is where autocomplete
// looks first anyway.

use serde::Deserialize;

use crate::dto::Person;
use crate::google::api::{self, ApiError};

const BASE: &str = "https://people.googleapis.com/v1";

pub const SCOPE_CONTACTS: &str = "https://www.googleapis.com/auth/contacts.readonly";
pub const SCOPE_OTHER_CONTACTS: &str = "https://www.googleapis.com/auth/contacts.other.readonly";

/// The maximum both endpoints accept.
const PAGE_SIZE: &str = "1000";

/// Only what an address book row needs. Asking for more is the difference between a scope review
/// that goes through and one that asks why a mail client wants birthdays.
const FIELDS: &str = "names,emailAddresses";

/// A page is capped at 1,000 and a large `otherContacts` runs to several. The cap is a guard
/// against paging forever on a mailbox with a pathological contact list, not a product decision.
const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Name {
    #[serde(default)]
    display_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct EmailAddress {
    #[serde(default)]
    value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPerson {
    #[serde(default)]
    names: Vec<Name>,
    #[serde(default)]
    email_addresses: Vec<EmailAddress>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionsPage {
    #[serde(default)]
    connections: Vec<RawPerson>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtherContactsPage {
    #[serde(default)]
    other_contacts: Vec<RawPerson>,
    #[serde(default)]
    next_page_token: Option<String>,
}

/// One row per address rather than one per person: a contact with a work and a home address is two
/// autocomplete entries, and the name is shared between them.
fn flatten(people: &[RawPerson]) -> Vec<Person> {
    let mut out = Vec::new();
    for person in people {
        let name = person
            .names
            .iter()
            .map(|n| n.display_name.trim())
            .find(|n| !n.is_empty())
            .map(|n| n.to_string());
        for email in &person.email_addresses {
            let address = email.value.trim();
            if address.is_empty() {
                continue;
            }
            out.push(Person {
                name: name.clone(),
                address: address.to_ascii_lowercase(),
            });
        }
    }
    out
}

/// Last name wins for a duplicated address, which is what a person expects after they have renamed
/// somebody in their address book and the auto-collected copy still has the old spelling.
pub fn merge(lists: Vec<Vec<Person>>) -> Vec<Person> {
    let mut order: Vec<String> = Vec::new();
    let mut by_address: std::collections::HashMap<String, Person> = std::collections::HashMap::new();
    for list in lists {
        for person in list {
            if !by_address.contains_key(&person.address) {
                order.push(person.address.clone());
            }
            let named = person.name.is_some();
            match by_address.get(&person.address) {
                Some(existing) if existing.name.is_some() && !named => continue,
                _ => {
                    by_address.insert(person.address.clone(), person);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|address| by_address.remove(&address))
        .collect()
}

/// The address book the user curated.
pub async fn connections(access_token: &str) -> Result<Vec<Person>, ApiError> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut params = vec![("personFields", FIELDS), ("pageSize", PAGE_SIZE)];
        if let Some(token) = &page_token {
            params.push(("pageToken", token.as_str()));
        }
        let resp = api::HTTP
            .get(api::url_with(
                &format!("{BASE}/people/me/connections"),
                &params,
            ))
            .bearer_auth(access_token)
            .send()
            .await?;
        let page: ConnectionsPage =
            api::read_json(resp, "Google contacts", SCOPE_CONTACTS).await?;
        out.extend(flatten(&page.connections));
        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }
    Ok(out)
}

/// The auto-collected set, which is the one Gmail's own autocomplete runs on. `readMask`, not
/// `personFields`: the two endpoints spell the same parameter differently.
pub async fn other_contacts(access_token: &str) -> Result<Vec<Person>, ApiError> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut params = vec![("readMask", FIELDS), ("pageSize", PAGE_SIZE)];
        if let Some(token) = &page_token {
            params.push(("pageToken", token.as_str()));
        }
        let resp = api::HTTP
            .get(api::url_with(&format!("{BASE}/otherContacts"), &params))
            .bearer_auth(access_token)
            .send()
            .await?;
        let page: OtherContactsPage =
            api::read_json(resp, "Google contacts", SCOPE_OTHER_CONTACTS).await?;
        out.extend(flatten(&page.other_contacts));
        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }
    Ok(out)
}

/// Both lists, merged. A user who cleared one of the two tick boxes on the consent screen still
/// gets the other, because half an address book beats an error message.
pub async fn all(access_token: &str) -> Result<Vec<Person>, ApiError> {
    let curated = connections(access_token).await;
    let collected = other_contacts(access_token).await;
    match (curated, collected) {
        (Ok(a), Ok(b)) => Ok(merge(vec![a, b])),
        (Ok(a), Err(ApiError::InsufficientScope(_))) => Ok(merge(vec![a])),
        (Err(ApiError::InsufficientScope(_)), Ok(b)) => Ok(merge(vec![b])),
        (Err(e), _) => Err(e),
        (_, Err(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OTHER_CONTACTS: &str = r#"{
      "otherContacts": [
        {"resourceName":"otherContacts/c123","names":[{"displayName":"Ana Ruiz"}],
         "emailAddresses":[{"value":"Ana@Example.com"},{"value":"ana.ruiz@work.example"}]},
        {"resourceName":"otherContacts/c124",
         "emailAddresses":[{"value":"receipts@shop.example"}]},
        {"resourceName":"otherContacts/c125","names":[{"displayName":"  "}],
         "emailAddresses":[{"value":"  "}]}
      ],
      "nextPageToken": ""
    }"#;

    #[test]
    fn a_contact_becomes_one_row_per_address_with_the_name_on_both() {
        let page: OtherContactsPage = serde_json::from_str(OTHER_CONTACTS).expect("the capture");
        let people = flatten(&page.other_contacts);
        assert_eq!(people.len(), 3);
        assert_eq!(people[0].name.as_deref(), Some("Ana Ruiz"));
        assert_eq!(people[0].address, "ana@example.com");
        assert_eq!(people[1].name.as_deref(), Some("Ana Ruiz"));
        assert_eq!(people[2].name, None);
        assert_eq!(people[2].address, "receipts@shop.example");
    }

    #[test]
    fn merging_keeps_the_first_sighting_of_an_address_and_the_better_name() {
        let curated = vec![Person {
            name: Some("Ana Ruiz".into()),
            address: "ana@example.com".into(),
        }];
        let collected = vec![
            Person {
                name: None,
                address: "ana@example.com".into(),
            },
            Person {
                name: Some("Shop".into()),
                address: "receipts@shop.example".into(),
            },
        ];
        let merged = merge(vec![curated, collected]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].address, "ana@example.com");
        assert_eq!(merged[0].name.as_deref(), Some("Ana Ruiz"));
        assert_eq!(merged[1].address, "receipts@shop.example");
    }
}
