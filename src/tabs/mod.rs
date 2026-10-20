pub mod calendar;
pub mod dashboard;
pub mod files;
pub mod mail;
pub mod messages;
pub mod notes;
pub mod settings;

use chrono::{Datelike, Weekday};

pub fn weekday_fr(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "lundi",
        Weekday::Tue => "mardi",
        Weekday::Wed => "mercredi",
        Weekday::Thu => "jeudi",
        Weekday::Fri => "vendredi",
        Weekday::Sat => "samedi",
        Weekday::Sun => "dimanche",
    }
}

pub fn month_fr(m: u32) -> &'static str {
    [
        "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre",
        "octobre", "novembre", "décembre",
    ][(m as usize).saturating_sub(1) % 12]
}

pub fn today_long() -> String {
    let d = chrono::Local::now().date_naive();
    format!("{} {} {} {}", weekday_fr(d.weekday()), d.day(), month_fr(d.month()), d.year())
}
