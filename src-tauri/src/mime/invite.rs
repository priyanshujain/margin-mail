// `text/calendar` with `METHOD:REQUEST`, read into the card the pane shows.
//
// Hand written rather than pulled in, because what an invite mail carries is a handful of
// properties out of one VEVENT and one VTIMEZONE, and the whole of RFC 5545 is recurrence rules,
// alarms, journals and free/busy that this app has no use for. The one thing worth being careful
// about is the instant: a wrong summary is a wrong label, a wrong instant is a missed lesson.
//
// The timezone comes from the VTIMEZONE the sender put in the same file, never from a lookup: a
// `TZID` is only a name, and this crate has no zone database to resolve it against. When the
// sender left the VTIMEZONE out there is nothing to resolve with, so the local time is read as UTC
// and the card is an hour or two out rather than absent. Senders that leave it out are rare and
// senders that leave it out and are not already UTC are rarer.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};

use crate::dto::{Invite, InviteResponse, Person};

pub fn parse(ics: &str, own_addresses: &[String]) -> Result<Option<Invite>, String> {
    let lines = unfold(ics);
    let props: Vec<Prop> = lines.iter().filter_map(|line| parse_prop(line)).collect();

    let mut method = String::new();
    let mut zones: Vec<Zone> = Vec::new();
    let mut event: Vec<Prop> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut have_event = false;

    for prop in props {
        match prop.name.as_str() {
            "BEGIN" => {
                let component = prop.value.to_ascii_uppercase();
                if component == "VTIMEZONE" {
                    zones.push(Zone::default());
                }
                if component == "STANDARD" || component == "DAYLIGHT" {
                    if let Some(zone) = zones.last_mut() {
                        zone.observances.push(Observance::default());
                    }
                }
                stack.push(component);
                continue;
            }
            "END" => {
                if prop.value.eq_ignore_ascii_case("VEVENT") {
                    have_event = true;
                }
                stack.pop();
                continue;
            }
            "METHOD" if stack.last().map(String::as_str) == Some("VCALENDAR") => {
                method = prop.value.trim().to_ascii_uppercase();
                continue;
            }
            _ => {}
        }

        match stack.last().map(String::as_str) {
            Some("VEVENT") if !have_event => event.push(prop),
            Some("VTIMEZONE") => {
                if prop.name == "TZID" {
                    if let Some(zone) = zones.last_mut() {
                        zone.id = prop.value.trim().to_string();
                    }
                }
            }
            Some("STANDARD") | Some("DAYLIGHT") => {
                if let Some(observance) = zones.last_mut().and_then(|zone| zone.observances.last_mut())
                {
                    observance.absorb(&prop);
                }
            }
            _ => {}
        }
    }

    if method != "REQUEST" || event.is_empty() {
        return Ok(None);
    }

    let value = |name: &str| {
        event
            .iter()
            .find(|prop| prop.name == name)
            .map(|prop| unescape(&prop.value))
    };
    let property = |name: &str| event.iter().find(|prop| prop.name == name);

    let start_prop = property("DTSTART").ok_or("the invite has no DTSTART")?;
    let (start_ms, all_day) = instant(start_prop, &zones)?;
    let end_ms = match property("DTEND") {
        Some(end) => instant(end, &zones)?.0,
        None => match value("DURATION").and_then(|text| duration_ms(&text)) {
            Some(length) => start_ms + length,
            None if all_day => start_ms + 86_400_000,
            None => start_ms,
        },
    };

    let organizer = property("ORGANIZER").map(|prop| Person {
        name: prop.param("CN").as_deref().map(unescape),
        address: address_of(&prop.value),
    });

    let my_response = event
        .iter()
        .filter(|prop| prop.name == "ATTENDEE")
        .find(|prop| {
            let address = address_of(&prop.value);
            own_addresses
                .iter()
                .any(|own| own.eq_ignore_ascii_case(&address))
        })
        .and_then(|prop| prop.param("PARTSTAT"))
        .map(|status| match status.to_ascii_uppercase().as_str() {
            "ACCEPTED" => InviteResponse::Accepted,
            "TENTATIVE" => InviteResponse::Tentative,
            "DECLINED" => InviteResponse::Declined,
            _ => InviteResponse::NeedsAction,
        })
        .unwrap_or(InviteResponse::NeedsAction);

    Ok(Some(Invite {
        uid: value("UID").unwrap_or_default(),
        summary: value("SUMMARY").unwrap_or_default(),
        start_ms,
        end_ms,
        all_day,
        location: value("LOCATION").filter(|text| !text.is_empty()),
        organizer,
        description: value("DESCRIPTION").filter(|text| !text.is_empty()),
        my_response,
        // The deep link is the calendar app's to mint, and it needs the account this landed in.
        calendar_link: None,
    }))
}

// -------------------------------------------------------------------------------------------
// Lines and properties
// -------------------------------------------------------------------------------------------

struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, name: &str) -> Option<String> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}

/// A continuation line is one that starts with a space or a tab, and the character that marks it is
/// not part of the value.
fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in ics.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        match line.strip_prefix([' ', '\t']) {
            Some(continuation) => {
                if let Some(previous) = lines.last_mut() {
                    previous.push_str(continuation);
                    continue;
                }
                lines.push(continuation.to_string());
            }
            None => lines.push(line.to_string()),
        }
    }
    lines
}

fn parse_prop(line: &str) -> Option<Prop> {
    if line.trim().is_empty() {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != b';' && bytes[i] != b':' {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let name = line[..i].trim().to_ascii_uppercase();

    let mut params = Vec::new();
    while i < bytes.len() && bytes[i] == b';' {
        i += 1;
        let start = i;
        let mut quoted = false;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => quoted = !quoted,
                b';' | b':' if !quoted => break,
                _ => {}
            }
            i += 1;
        }
        if let Some((key, value)) = line[start..i].split_once('=') {
            params.push((
                key.trim().to_ascii_uppercase(),
                value.trim().trim_matches('"').to_string(),
            ));
        }
    }

    let value = match bytes.get(i) {
        Some(b':') => line[i + 1..].to_string(),
        _ => String::new(),
    };
    Some(Prop {
        name,
        params,
        value,
    })
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(escaped) => out.push(escaped),
            None => out.push('\\'),
        }
    }
    out.trim().to_string()
}

fn address_of(value: &str) -> String {
    value
        .trim()
        .strip_prefix("mailto:")
        .or_else(|| value.trim().strip_prefix("MAILTO:"))
        .unwrap_or(value.trim())
        .to_string()
}

// -------------------------------------------------------------------------------------------
// Instants
// -------------------------------------------------------------------------------------------

#[derive(Default)]
struct Zone {
    id: String,
    observances: Vec<Observance>,
}

#[derive(Default)]
struct Observance {
    offset_seconds: i32,
    starts: Option<NaiveDateTime>,
    month: Option<u32>,
    weekday: Option<(i32, Weekday)>,
}

impl Observance {
    fn absorb(&mut self, prop: &Prop) {
        match prop.name.as_str() {
            "TZOFFSETTO" => self.offset_seconds = offset_seconds(&prop.value).unwrap_or(0),
            "DTSTART" => self.starts = naive(&prop.value),
            "RRULE" => {
                for part in prop.value.split(';') {
                    let Some((key, value)) = part.split_once('=') else {
                        continue;
                    };
                    match key.to_ascii_uppercase().as_str() {
                        "BYMONTH" => self.month = value.trim().parse().ok(),
                        "BYDAY" => self.weekday = parse_byday(value.trim()),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// When this observance takes over in the given year.
    fn transition(&self, year: i32) -> Option<NaiveDateTime> {
        let starts = self.starts?;
        let (Some(month), Some((ordinal, weekday))) = (self.month, self.weekday) else {
            return Some(starts);
        };
        nth_weekday(year, month, ordinal, weekday).map(|date| date.and_time(starts.time()))
    }
}

fn parse_byday(value: &str) -> Option<(i32, Weekday)> {
    let split = value.len().checked_sub(2)?;
    let (ordinal, day) = value.split_at(split);
    let weekday = match day.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    };
    Some((ordinal.parse().unwrap_or(1), weekday))
}

fn nth_weekday(year: i32, month: u32, ordinal: i32, weekday: Weekday) -> Option<NaiveDate> {
    if ordinal < 0 {
        let mut date = NaiveDate::from_ymd_opt(year, month, 1)?
            .checked_add_months(chrono::Months::new(1))?
            .pred_opt()?;
        let mut remaining = -ordinal;
        loop {
            if date.weekday() == weekday {
                remaining -= 1;
                if remaining == 0 {
                    return Some(date);
                }
            }
            date = date.pred_opt()?;
        }
    }
    let mut date = NaiveDate::from_ymd_opt(year, month, 1)?;
    let mut remaining = ordinal.max(1);
    loop {
        if date.weekday() == weekday {
            remaining -= 1;
            if remaining == 0 {
                return Some(date);
            }
        }
        date = date.succ_opt()?;
    }
}

fn offset_seconds(value: &str) -> Option<i32> {
    let value = value.trim();
    let sign = match value.as_bytes().first()? {
        b'-' => -1,
        b'+' => 1,
        _ => return None,
    };
    let digits = &value[1..];
    let hours: i32 = digits.get(0..2)?.parse().ok()?;
    let minutes: i32 = digits.get(2..4)?.parse().ok()?;
    let seconds: i32 = digits.get(4..6).and_then(|text| text.parse().ok()).unwrap_or(0);
    Some(sign * (hours * 3600 + minutes * 60 + seconds))
}

fn naive(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim().trim_end_matches('Z');
    match value.len() {
        8 => NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .and_then(|date| date.and_hms_opt(0, 0, 0)),
        15 => NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok(),
        _ => None,
    }
}

/// Epoch milliseconds, and whether the property was a date without a time.
fn instant(prop: &Prop, zones: &[Zone]) -> Result<(i64, bool), String> {
    let raw = prop.value.trim();
    let all_day = prop
        .param("VALUE")
        .map(|value| value.eq_ignore_ascii_case("DATE"))
        .unwrap_or(false)
        || raw.len() == 8;
    let local = naive(raw).ok_or_else(|| format!("{} is not a date: {raw:?}", prop.name))?;

    let offset = if raw.ends_with('Z') || all_day {
        0
    } else {
        match prop.param("TZID") {
            Some(tzid) => offset_for(&tzid, local, zones),
            None => 0,
        }
    };
    Ok((
        (local.and_utc().timestamp() - offset as i64) * 1000,
        all_day,
    ))
}

fn offset_for(tzid: &str, local: NaiveDateTime, zones: &[Zone]) -> i32 {
    let Some(zone) = zones.iter().find(|zone| zone.id.eq_ignore_ascii_case(tzid)) else {
        return 0;
    };
    match zone.observances.len() {
        0 => 0,
        1 => zone.observances[0].offset_seconds,
        _ => {
            let mut transitions: Vec<(NaiveDateTime, i32)> = zone
                .observances
                .iter()
                .filter_map(|observance| {
                    observance
                        .transition(local.year())
                        .map(|at| (at, observance.offset_seconds))
                })
                .collect();
            transitions.sort_by_key(|(at, _)| *at);
            match transitions.iter().rev().find(|(at, _)| *at <= local) {
                Some((_, offset)) => *offset,
                // Before the first transition of the year, the observance in force is the one the
                // previous year ended in, which is the last one in the list.
                None => transitions.last().map(|(_, offset)| *offset).unwrap_or(0),
            }
        }
    }
}

fn duration_ms(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, rest) = match value.as_bytes().first()? {
        b'-' => (-1i64, &value[1..]),
        b'+' => (1, &value[1..]),
        _ => (1, value),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    for character in rest.chars() {
        match character {
            'T' => in_time = true,
            '0'..='9' => number.push(character),
            unit => {
                let count: i64 = number.parse().ok()?;
                number.clear();
                total += count
                    * match (unit, in_time) {
                        ('W', _) => 604_800,
                        ('D', _) => 86_400,
                        ('H', true) => 3_600,
                        ('M', true) => 60,
                        ('S', true) => 1,
                        _ => return None,
                    };
            }
        }
    }
    Some(sign * total * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIANO: &str = "\
BEGIN:VCALENDAR\r
METHOD:REQUEST\r
BEGIN:VTIMEZONE\r
TZID:Asia/Kolkata\r
BEGIN:STANDARD\r
TZOFFSETFROM:+0530\r
TZOFFSETTO:+0530\r
TZNAME:IST\r
DTSTART:19700101T000000\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
DTSTART;TZID=Asia/Kolkata:20260909T170000\r
DTEND;TZID=Asia/Kolkata:20260909T174500\r
UID:4c9b2f7a-piano-cooper@sunnydaymusic.example\r
ORGANIZER;CN=Sunny Day Music:mailto:lessons@sunnydaymusic.example\r
ATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Priyanshu Jain:mailto:pj@73ai.org\r
DESCRIPTION:First lesson for Cooper. Nothing to prepare\\, and there is a pia\r
 no here.\r
LOCATION:Sunny Day Music\\, Bandra\r
SUMMARY:Piano lesson: Cooper\r
END:VEVENT\r
END:VCALENDAR\r
";

    fn piano() -> Invite {
        parse(PIANO, &["pj@73ai.org".to_string()])
            .expect("a parse")
            .expect("an invite")
    }

    #[test]
    fn a_tzid_is_resolved_from_the_senders_own_vtimezone() {
        // 2026-09-09T17:00:00+05:30 is 2026-09-09T11:30:00Z.
        assert_eq!(piano().start_ms, 1_788_953_400_000);
        assert_eq!(piano().end_ms, 1_788_953_400_000 + 45 * 60 * 1000);
        assert!(!piano().all_day);
    }

    #[test]
    fn the_text_values_are_unescaped_and_the_folding_is_undone() {
        let invite = piano();
        assert_eq!(invite.summary, "Piano lesson: Cooper");
        assert_eq!(invite.location.as_deref(), Some("Sunny Day Music, Bandra"));
        assert_eq!(
            invite.description.as_deref(),
            Some("First lesson for Cooper. Nothing to prepare, and there is a piano here.")
        );
        assert_eq!(invite.uid, "4c9b2f7a-piano-cooper@sunnydaymusic.example");
    }

    #[test]
    fn the_organizer_and_this_accounts_own_answer_come_out() {
        let invite = piano();
        let organizer = invite.organizer.expect("an organizer");
        assert_eq!(organizer.address, "lessons@sunnydaymusic.example");
        assert_eq!(organizer.name.as_deref(), Some("Sunny Day Music"));
        assert_eq!(invite.my_response, InviteResponse::NeedsAction);
    }

    #[test]
    fn another_accounts_answer_is_not_this_accounts_answer() {
        let invite = parse(
            &PIANO.replace("PARTSTAT=NEEDS-ACTION", "PARTSTAT=ACCEPTED"),
            &["someone.else@example.org".to_string()],
        )
        .expect("a parse")
        .expect("an invite");
        assert_eq!(invite.my_response, InviteResponse::NeedsAction);
    }

    #[test]
    fn a_date_without_a_time_is_an_all_day_event() {
        let ics = PIANO
            .replace(
                "DTSTART;TZID=Asia/Kolkata:20260909T170000",
                "DTSTART;VALUE=DATE:20260909",
            )
            .replace("DTEND;TZID=Asia/Kolkata:20260909T174500\r\n", "");
        let invite = parse(&ics, &[]).expect("a parse").expect("an invite");
        assert!(invite.all_day);
        assert_eq!(invite.end_ms - invite.start_ms, 86_400_000);
    }

    #[test]
    fn a_utc_stamp_needs_no_zone_at_all() {
        let ics = PIANO
            .replace(
                "DTSTART;TZID=Asia/Kolkata:20260909T170000",
                "DTSTART:20260909T113000Z",
            )
            .replace(
                "DTEND;TZID=Asia/Kolkata:20260909T174500",
                "DTEND:20260909T121500Z",
            );
        let invite = parse(&ics, &[]).expect("a parse").expect("an invite");
        assert_eq!(invite.start_ms, 1_788_953_400_000);
    }

    #[test]
    fn daylight_saving_picks_the_observance_in_force() {
        let ics = "\
BEGIN:VCALENDAR\r
METHOD:REQUEST\r
BEGIN:VTIMEZONE\r
TZID:Europe/London\r
BEGIN:DAYLIGHT\r
TZOFFSETTO:+0100\r
DTSTART:19810329T010000\r
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r
END:DAYLIGHT\r
BEGIN:STANDARD\r
TZOFFSETTO:+0000\r
DTSTART:19961027T020000\r
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r
END:STANDARD\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:x@example.org\r
SUMMARY:Summer\r
DTSTART;TZID=Europe/London:20260701T120000\r
DTEND;TZID=Europe/London:20260701T130000\r
END:VEVENT\r
END:VCALENDAR\r
";
        let invite = parse(ics, &[]).expect("a parse").expect("an invite");
        // Noon in July in London is 11:00 UTC, not 12:00.
        let noon_utc = NaiveDate::from_ymd_opt(2026, 7, 1)
            .and_then(|date| date.and_hms_opt(11, 0, 0))
            .expect("a date")
            .and_utc()
            .timestamp()
            * 1000;
        assert_eq!(invite.start_ms, noon_utc);
    }

    #[test]
    fn a_reply_is_not_a_request() {
        let ics = PIANO.replace("METHOD:REQUEST", "METHOD:REPLY");
        assert!(parse(&ics, &[]).expect("a parse").is_none());
    }

    #[test]
    fn a_duration_stands_in_for_a_missing_end() {
        let ics = PIANO.replace(
            "DTEND;TZID=Asia/Kolkata:20260909T174500",
            "DURATION:PT1H30M",
        );
        let invite = parse(&ics, &[]).expect("a parse").expect("an invite");
        assert_eq!(invite.end_ms - invite.start_ms, 90 * 60 * 1000);
    }
}
