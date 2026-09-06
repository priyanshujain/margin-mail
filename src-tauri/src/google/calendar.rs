// Enough of the Calendar API to answer an invite, and nothing else.
//
// This is the one feature in the app that needs a scope beyond mail and contacts, and installed
// apps get no incremental authorization, so `calendar.events` cannot be added to a live token: the
// first Accept re-runs the whole consent with it in the list. That is why this file is three calls
// rather than a calendar client, and why the app says what is about to happen before it starts.
//
// The shape of an RSVP, which is not obvious from the reference:
//
//   1. Find the event by the `UID` out of the message's `text/calendar` part. Gmail adds invites to
//      the user's calendar by itself, but only when their "Add invitations to my calendar" setting
//      says to, so it is there most of the time and not always.
//   2. If it is not there, `events.import` it, which is the only method that takes an `iCalUID` and
//      the one that exists for exactly this.
//   3. Patch this account's own attendee row. `events.patch` replaces the whole `attendees` array
//      rather than merging into it, so the array has to be read, changed and written back; sending
//      only the one attendee would silently drop everybody else off the invite.

use serde::{Deserialize, Serialize};

use crate::dto::{Invite, InviteResponse};
use crate::google::api::{self, ApiError};

const BASE: &str = "https://www.googleapis.com/calendar/v3";

pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";

/// The invite always lands on the account's own calendar, never on a secondary one.
const CALENDAR: &str = "primary";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "iCalUID")]
    pub ical_uid: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub html_link: Option<String>,
    /// Left as JSON: this file changes one field in it and hands the rest back untouched, and
    /// typing it would mean owning every attendee field Google adds.
    #[serde(default)]
    pub attendees: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsPage {
    #[serde(default)]
    items: Vec<Event>,
}

pub fn response_status(response: InviteResponse) -> &'static str {
    match response {
        InviteResponse::Accepted => "accepted",
        InviteResponse::Tentative => "tentative",
        InviteResponse::Declined => "declined",
        InviteResponse::NeedsAction => "needsAction",
    }
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string()
}

/// All-day events are a date with no zone. Shifting one into UTC is how a birthday ends up on the
/// day before in half the world.
fn moment(ms: i64, all_day: bool) -> serde_json::Value {
    if all_day {
        serde_json::json!({ "date": date(ms) })
    } else {
        serde_json::json!({ "dateTime": rfc3339(ms), "timeZone": "UTC" })
    }
}

/// This account's row in the attendee list, changed, with everybody else's left exactly as Google
/// sent them. An account that is not in the list is added to it, which happens when the invite
/// reached them through a mailing list or a forward.
pub fn with_response(
    attendees: Option<&serde_json::Value>,
    self_email: &str,
    response: InviteResponse,
) -> serde_json::Value {
    let status = response_status(response);
    let mut list: Vec<serde_json::Value> = attendees
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    let mine = list.iter_mut().find(|attendee| {
        attendee
            .get("email")
            .and_then(|e| e.as_str())
            .map(|email| email.eq_ignore_ascii_case(self_email))
            .unwrap_or(false)
            || attendee
                .get("self")
                .and_then(|s| s.as_bool())
                .unwrap_or(false)
    });

    match mine {
        Some(attendee) => {
            attendee["responseStatus"] = serde_json::Value::String(status.to_string());
        }
        None => list.push(serde_json::json!({
            "email": self_email,
            "self": true,
            "responseStatus": status,
        })),
    }
    serde_json::Value::Array(list)
}

/// The body for an invite Gmail did not add to the calendar by itself. `import` is the only method
/// that accepts an `iCalUID`, which is what keeps this event and the organiser's the same event.
pub fn import_body(
    invite: &Invite,
    self_email: &str,
    response: InviteResponse,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "iCalUID": invite.uid,
        "summary": invite.summary,
        "start": moment(invite.start_ms, invite.all_day),
        "end": moment(invite.end_ms, invite.all_day),
        "attendees": with_response(None, self_email, response),
    });
    if let Some(location) = &invite.location {
        body["location"] = serde_json::Value::String(location.clone());
    }
    if let Some(description) = &invite.description {
        body["description"] = serde_json::Value::String(description.clone());
    }
    if let Some(organizer) = &invite.organizer {
        let mut who = serde_json::json!({ "email": organizer.address });
        if let Some(name) = &organizer.name {
            who["displayName"] = serde_json::Value::String(name.clone());
        }
        body["organizer"] = who;
    }
    body
}

pub async fn find_by_ical_uid(
    access_token: &str,
    ical_uid: &str,
) -> Result<Option<Event>, ApiError> {
    let resp = api::HTTP
        .get(api::url_with(
            &format!("{BASE}/calendars/{CALENDAR}/events"),
            &[
                ("iCalUID", ical_uid),
                ("showDeleted", "true"),
                ("maxResults", "5"),
            ],
        ))
        .bearer_auth(access_token)
        .send()
        .await?;
    let page: EventsPage = api::read_json(resp, "Google calendar lookup", SCOPE).await?;
    Ok(page.items.into_iter().find(|e| e.status != "cancelled"))
}

pub async fn import(
    access_token: &str,
    invite: &Invite,
    self_email: &str,
    response: InviteResponse,
) -> Result<Event, ApiError> {
    let resp = api::HTTP
        .post(format!("{BASE}/calendars/{CALENDAR}/events/import"))
        .bearer_auth(access_token)
        .json(&import_body(invite, self_email, response))
        .send()
        .await?;
    api::read_json(resp, "Google calendar import", SCOPE).await
}

/// `sendUpdates=all` is what tells the organiser. Without it the answer lands on the user's own
/// calendar and nowhere else, which looks to the organiser like being ignored.
pub async fn patch_response(
    access_token: &str,
    event: &Event,
    self_email: &str,
    response: InviteResponse,
) -> Result<Event, ApiError> {
    let body = serde_json::json!({
        "attendees": with_response(event.attendees.as_ref(), self_email, response),
    });
    let resp = api::HTTP
        .patch(api::url_with(
            &format!(
                "{BASE}/calendars/{CALENDAR}/events/{}",
                api::path_segment(&event.id)
            ),
            &[("sendUpdates", "all")],
        ))
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;
    api::read_json(resp, "Google calendar response", SCOPE).await
}

/// The whole RSVP: find it, import it when Gmail did not, then answer.
pub async fn respond(
    access_token: &str,
    invite: &Invite,
    self_email: &str,
    response: InviteResponse,
) -> Result<Event, ApiError> {
    match find_by_ical_uid(access_token, &invite.uid).await? {
        Some(event) => patch_response(access_token, &event, self_email, response).await,
        // The import already carries the answer, so there is nothing left to patch.
        None => import(access_token, invite, self_email, response).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::Person;

    fn invite() -> Invite {
        Invite {
            uid: "040000008200E00074C5B7101A82E00800000000@example.com".into(),
            summary: "Lease signing".into(),
            start_ms: 1_785_808_800_000,
            end_ms: 1_785_812_400_000,
            all_day: false,
            location: Some("The office".into()),
            organizer: Some(Person {
                name: Some("Ana Ruiz".into()),
                address: "ana@example.com".into(),
            }),
            description: None,
            my_response: InviteResponse::NeedsAction,
            calendar_link: None,
        }
    }

    /// Two attendees and an organiser, as Google returns them.
    const ATTENDEES: &str = r#"[
      {"email":"ana@example.com","organizer":true,"responseStatus":"accepted"},
      {"email":"you@example.com","self":true,"responseStatus":"needsAction"},
      {"email":"sam@example.com","responseStatus":"tentative"}
    ]"#;

    #[test]
    fn answering_changes_one_attendee_and_leaves_the_rest_alone() {
        let attendees: serde_json::Value = serde_json::from_str(ATTENDEES).expect("the capture");
        let patched = with_response(Some(&attendees), "you@example.com", InviteResponse::Declined);
        let list = patched.as_array().expect("an array");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["responseStatus"], "accepted");
        assert_eq!(list[1]["responseStatus"], "declined");
        assert_eq!(list[2]["responseStatus"], "tentative");
        assert_eq!(list[1]["email"], "you@example.com");
    }

    #[test]
    fn a_case_different_address_is_still_this_account() {
        let attendees: serde_json::Value = serde_json::from_str(ATTENDEES).expect("the capture");
        let patched = with_response(Some(&attendees), "You@Example.COM", InviteResponse::Accepted);
        assert_eq!(patched[1]["responseStatus"], "accepted");
    }

    #[test]
    fn an_account_that_is_not_on_the_invite_is_added_rather_than_replacing_it() {
        let attendees: serde_json::Value = serde_json::from_str(
            r#"[{"email":"ana@example.com","organizer":true,"responseStatus":"accepted"}]"#,
        )
        .expect("one attendee");
        let patched = with_response(
            Some(&attendees),
            "forwarded@example.com",
            InviteResponse::Tentative,
        );
        let list = patched.as_array().expect("an array");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["email"], "ana@example.com");
        assert_eq!(list[1]["email"], "forwarded@example.com");
        assert_eq!(list[1]["responseStatus"], "tentative");
    }

    #[test]
    fn an_imported_invite_keeps_the_organisers_uid_so_it_is_the_same_event() {
        let body = import_body(&invite(), "you@example.com", InviteResponse::Accepted);
        assert_eq!(
            body["iCalUID"],
            "040000008200E00074C5B7101A82E00800000000@example.com"
        );
        assert_eq!(body["start"]["dateTime"], "2026-08-04T02:00:00Z");
        assert_eq!(body["end"]["dateTime"], "2026-08-04T03:00:00Z");
        assert_eq!(body["organizer"]["email"], "ana@example.com");
        assert_eq!(body["attendees"][0]["responseStatus"], "accepted");
        assert_eq!(body["location"], "The office");
    }

    #[test]
    fn an_all_day_invite_is_a_date_with_no_zone() {
        let mut invite = invite();
        invite.all_day = true;
        let body = import_body(&invite, "you@example.com", InviteResponse::Tentative);
        assert_eq!(body["start"], serde_json::json!({ "date": "2026-08-04" }));
        assert!(body["start"].get("timeZone").is_none());
    }

    #[test]
    fn the_four_answers_are_googles_four_words() {
        assert_eq!(response_status(InviteResponse::Accepted), "accepted");
        assert_eq!(response_status(InviteResponse::Tentative), "tentative");
        assert_eq!(response_status(InviteResponse::Declined), "declined");
        assert_eq!(response_status(InviteResponse::NeedsAction), "needsAction");
    }
}
