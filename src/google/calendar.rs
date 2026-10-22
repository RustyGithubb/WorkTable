//! API Google Agenda v3 : agendas, événements (lecture, création, modification,
//! déplacement entre agendas, suppression).

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use serde_json::{Value, json};

use super::Session;

const API: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Clone, Debug, PartialEq)]
pub struct CalendarInfo {
    pub id: String,
    pub name: String,
    pub color: [u8; 3],
    pub primary: bool,
    /// Droit d'écriture (owner / writer).
    pub writable: bool,
    /// Affiché par défaut dans Google Agenda.
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CalEvent {
    pub id: String,
    pub calendar: String,
    pub title: String,
    pub description: String,
    pub location: String,
    /// Début / fin en millisecondes (fin exclue). Journée entière : minuit local.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub attendees: Vec<String>,
    pub meet_link: Option<String>,
    pub html_link: Option<String>,
    /// Couleur propre à l'événement (sinon celle de l'agenda).
    pub color: Option<[u8; 3]>,
}

/// Couleurs d'événement de Google Agenda (colorId 1 à 11).
pub const EVENT_COLORS: [[u8; 3]; 11] = [
    [0x79, 0x86, 0xCB], // 1 lavande
    [0x33, 0xB6, 0x79], // 2 sauge
    [0x8E, 0x24, 0xAA], // 3 raisin
    [0xE6, 0x7C, 0x73], // 4 flamant
    [0xF6, 0xBF, 0x26], // 5 banane
    [0xF4, 0x51, 0x1E], // 6 mandarine
    [0x03, 0x9B, 0xE5], // 7 paon
    [0x61, 0x61, 0x61], // 8 graphite
    [0x3F, 0x51, 0xB5], // 9 myrtille
    [0x0B, 0x80, 0x43], // 10 basilic
    [0xD5, 0x00, 0x00], // 11 tomate
];

fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

pub fn rfc3339(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.to_rfc3339()).unwrap_or_default()
}

fn date_str(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// `{"dateTime": …}` ou `{"date": …}` → (ms, journée entière ?)
fn parse_time(v: &Value) -> Option<(i64, bool)> {
    if let Some(dt) = v["dateTime"].as_str() {
        return DateTime::parse_from_rfc3339(dt).ok().map(|d| (d.timestamp_millis(), false));
    }
    let d = NaiveDate::parse_from_str(v["date"].as_str()?, "%Y-%m-%d").ok()?;
    let local = Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest()?;
    Some((local.timestamp_millis(), true))
}

pub fn parse_event(calendar: &str, v: &Value) -> Option<CalEvent> {
    if v["status"].as_str() == Some("cancelled") {
        return None;
    }
    let (start, all_day) = parse_time(&v["start"])?;
    let (end, _) = parse_time(&v["end"]).unwrap_or((start + 3_600_000, all_day));
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    Some(CalEvent {
        id: s("id"),
        calendar: calendar.to_string(),
        title: s("summary"),
        description: s("description"),
        location: s("location"),
        start,
        end: end.max(start + 15 * 60_000),
        all_day,
        attendees: v["attendees"]
            .as_array()
            .map(|a| a.iter().filter(|x| !x["self"].as_bool().unwrap_or(false)).filter_map(|x| x["email"].as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        meet_link: v["hangoutLink"]
            .as_str()
            .map(str::to_string)
            .or_else(|| v["conferenceData"]["entryPoints"].as_array()?.iter().find(|e| e["entryPointType"] == "video")?["uri"].as_str().map(str::to_string)),
        html_link: v["htmlLink"].as_str().map(str::to_string),
        color: v["colorId"].as_str().and_then(|c| c.parse::<usize>().ok()).and_then(|i| EVENT_COLORS.get(i.wrapping_sub(1)).copied()),
    })
}

pub fn calendars(s: &Session) -> Result<Vec<CalendarInfo>, String> {
    let v = s.get(&format!("{API}/users/me/calendarList"), &[("maxResults", "250")])?;
    let mut out: Vec<CalendarInfo> = v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| CalendarInfo {
            id: c["id"].as_str().unwrap_or_default().to_string(),
            name: c["summaryOverride"].as_str().or(c["summary"].as_str()).unwrap_or_default().to_string(),
            color: c["backgroundColor"].as_str().and_then(hex).unwrap_or([0x4C, 0x9A, 0xFF]),
            primary: c["primary"].as_bool().unwrap_or(false),
            writable: matches!(c["accessRole"].as_str(), Some("owner" | "writer")),
            selected: c["selected"].as_bool().unwrap_or(false),
        })
        .collect();
    // Agenda principal d'abord, puis ordre alphabétique.
    out.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// Événements d'un agenda entre `from` et `to` (ms), récurrences développées.
pub fn events(s: &Session, calendar: &str, from: i64, to: i64) -> Result<Vec<CalEvent>, String> {
    let (min, max) = (rfc3339(from), rfc3339(to));
    let url = format!("{API}/calendars/{}/events", enc(calendar));
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut q = vec![("timeMin", min.as_str()), ("timeMax", max.as_str()), ("singleEvents", "true"), ("orderBy", "startTime"), ("maxResults", "2500")];
        if let Some(p) = &page {
            q.push(("pageToken", p.as_str()));
        }
        let v = s.get(&url, &q)?;
        out.extend(v["items"].as_array().into_iter().flatten().filter_map(|e| parse_event(calendar, e)));
        page = v["nextPageToken"].as_str().map(str::to_string);
        if page.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Corps JSON d'un événement (création / modification).
pub fn body(e: &CalEvent, add_meet: bool) -> Value {
    let time = |ms: i64| {
        if e.all_day { json!({ "date": date_str(ms) }) } else { json!({ "dateTime": rfc3339(ms) }) }
    };
    let mut b = json!({
        "summary": e.title,
        "description": e.description,
        "location": e.location,
        "start": time(e.start),
        "end": time(e.end),
        "attendees": e.attendees.iter().map(|a| json!({ "email": a })).collect::<Vec<_>>(),
    });
    if let Some(c) = e.color.and_then(|c| EVENT_COLORS.iter().position(|x| *x == c)) {
        b["colorId"] = json!((c + 1).to_string());
    }
    if add_meet && e.meet_link.is_none() {
        b["conferenceData"] = json!({
            "createRequest": {
                "requestId": format!("wt-{}", chrono::Utc::now().timestamp_millis()),
                "conferenceSolutionKey": { "type": "hangoutsMeet" }
            }
        });
    }
    b
}

fn notify(e: &CalEvent) -> &'static str {
    if e.attendees.is_empty() { "none" } else { "all" }
}

pub fn create(s: &Session, e: &CalEvent, add_meet: bool) -> Result<CalEvent, String> {
    let url = format!("{API}/calendars/{}/events", enc(&e.calendar));
    let v = s.send("POST", &url, &[("conferenceDataVersion", "1"), ("sendUpdates", notify(e))], Some(body(e, add_meet)))?;
    parse_event(&e.calendar, &v).ok_or_else(|| t!("Réponse inattendue de Google Agenda", "Unexpected response from Google Calendar").into())
}

/// Modifie un événement ; s'il change d'agenda, il est d'abord déplacé.
pub fn update(s: &Session, e: &CalEvent, from_calendar: &str, add_meet: bool) -> Result<CalEvent, String> {
    if from_calendar != e.calendar {
        let url = format!("{API}/calendars/{}/events/{}/move", enc(from_calendar), enc(&e.id));
        s.send("POST", &url, &[("destination", e.calendar.as_str())], None)?;
    }
    let url = format!("{API}/calendars/{}/events/{}", enc(&e.calendar), enc(&e.id));
    let v = s.send("PATCH", &url, &[("conferenceDataVersion", "1"), ("sendUpdates", notify(e))], Some(body(e, add_meet)))?;
    parse_event(&e.calendar, &v).ok_or_else(|| t!("Réponse inattendue de Google Agenda", "Unexpected response from Google Calendar").into())
}

pub fn delete(s: &Session, calendar: &str, id: &str, notify_guests: bool) -> Result<(), String> {
    let url = format!("{API}/calendars/{}/events/{}", enc(calendar), enc(id));
    s.send("DELETE", &url, &[("sendUpdates", if notify_guests { "all" } else { "none" })], None).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timed_and_all_day_events() {
        let v = json!({
            "id": "e1", "summary": "Réunion", "status": "confirmed",
            "start": { "dateTime": "2026-10-21T10:00:00+02:00" },
            "end": { "dateTime": "2026-10-21T11:30:00+02:00" },
            "attendees": [{ "email": "moi@artfx.fr", "self": true }, { "email": "lea@studio.fr" }],
            "hangoutLink": "https://meet.google.com/abc", "colorId": "7"
        });
        let e = parse_event("primary", &v).unwrap();
        assert_eq!(e.end - e.start, 90 * 60_000);
        assert!(!e.all_day);
        assert_eq!(e.attendees, vec!["lea@studio.fr"]);
        assert_eq!(e.color, Some(EVENT_COLORS[6]));
        assert_eq!(e.meet_link.as_deref(), Some("https://meet.google.com/abc"));

        let v = json!({ "id": "e2", "start": { "date": "2026-10-21" }, "end": { "date": "2026-10-22" } });
        let e = parse_event("primary", &v).unwrap();
        assert!(e.all_day);
        assert_eq!(e.end - e.start, 86_400_000);
        assert!(parse_event("p", &json!({ "status": "cancelled", "start": { "date": "2026-10-21" } })).is_none());

        // Aller-retour vers le JSON de l'API.
        let b = body(&e, false);
        assert_eq!(b["start"]["date"], "2026-10-21");
        assert_eq!(b["end"]["date"], "2026-10-22");
    }

    #[test]
    fn colors() {
        assert_eq!(hex("#9fe1e7"), Some([0x9f, 0xe1, 0xe7]));
        assert_eq!(hex("bad"), None);
    }
}
