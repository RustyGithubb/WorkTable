//! Tâches du tableau de bord (petit Kanban) : À faire → En cours → Archivé.
//! Le bloc « Archivé » repart à zéro chaque jour : les tâches archivées les jours précédents
//! passent dans l'historique des archives. Sauvegarde locale : tasks.json.

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Todo, Status::Doing, Status::Done];

    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => t!("À faire", "To do"),
            Status::Doing => t!("En cours", "In progress"),
            Status::Done => t!("Archivé", "Archived"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: Status,
    /// Création (ms).
    pub created: i64,
    /// Entrée dans l'état actuel (ms) : « en cours depuis… ».
    pub since: i64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Tasks {
    pub list: Vec<Task>,
    /// Tâches archivées les jours précédents (plus récentes en premier).
    pub archive: Vec<Task>,
    next_id: u64,
    #[serde(skip)]
    dirty: bool,
}

/// Une tâche à faire depuis plus longtemps est rappelée (jours).
pub const TODO_REMIND_DAYS: i64 = 1;
/// Une tâche en cours depuis plus longtemps est rappelée (jours).
pub const DOING_REMIND_DAYS: i64 = 3;
const DAY_MS: i64 = 86_400_000;

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("tasks.json"))
}

fn now() -> i64 {
    Local::now().timestamp_millis()
}

fn day_of(ms: i64) -> chrono::NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

impl Tasks {
    pub fn load() -> Self {
        let mut t: Tasks = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        t.roll_over(now());
        t
    }

    pub fn save_if_needed(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(p) = path() {
            crate::paths::save_json(&p, self);
        }
    }

    /// Remise à zéro quotidienne du bloc « Archivé » (vers l'historique).
    pub fn roll_over(&mut self, at: i64) {
        let today = day_of(at);
        let (old, keep): (Vec<Task>, Vec<Task>) = std::mem::take(&mut self.list).into_iter().partition(|t| t.status == Status::Done && day_of(t.since) < today);
        self.list = keep;
        if !old.is_empty() {
            let mut old = old;
            old.sort_by_key(|t| -t.since);
            for t in old.into_iter().rev() {
                self.archive.insert(0, t);
            }
            self.archive.sort_by_key(|t| -t.since);
            self.dirty = true;
        }
    }

    pub fn add(&mut self, title: &str) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        self.next_id = self.next_id.max(self.list.iter().chain(&self.archive).map(|t| t.id).max().unwrap_or(0)) + 1;
        let n = now();
        self.list.push(Task { id: self.next_id, title: title.to_string(), status: Status::Todo, created: n, since: n });
        self.dirty = true;
    }

    pub fn set_status(&mut self, id: u64, status: Status) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id)
            && t.status != status
        {
            t.status = status;
            t.since = now();
            self.dirty = true;
        }
    }

    pub fn rename(&mut self, id: u64, title: &str) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id) {
            t.title = title.trim().to_string();
            self.dirty = true;
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.list.retain(|t| t.id != id);
        self.archive.retain(|t| t.id != id);
        self.dirty = true;
    }

    /// Remet une tâche de l'historique dans « À faire ».
    pub fn restore(&mut self, id: u64) {
        if let Some(i) = self.archive.iter().position(|t| t.id == id) {
            let mut t = self.archive.remove(i);
            t.status = Status::Todo;
            t.since = now();
            self.list.push(t);
            self.dirty = true;
        }
    }

    pub fn column(&self, s: Status) -> Vec<&Task> {
        let mut v: Vec<&Task> = self.list.iter().filter(|t| t.status == s).collect();
        v.sort_by_key(|t| t.since);
        v
    }

    /// Tâches oubliées : à faire depuis les jours précédents, ou en cours depuis trop longtemps.
    pub fn reminders(&self, at: i64) -> Vec<(&Task, i64)> {
        let mut out: Vec<(&Task, i64)> = self
            .list
            .iter()
            .filter_map(|t| {
                let days = (at - t.since) / DAY_MS;
                match t.status {
                    Status::Todo if day_of(t.since) < day_of(at) && days >= TODO_REMIND_DAYS - 1 => Some((t, days.max(1))),
                    Status::Doing if days >= DOING_REMIND_DAYS => Some((t, days)),
                    _ => None,
                }
            })
            .collect();
        out.sort_by_key(|(t, d)| (-d, t.id as i64));
        out
    }
}

/// « aujourd'hui », « hier », « il y a 3 jours ».
pub fn ago(days: i64) -> String {
    match days {
        0 => t!("aujourd'hui", "today").into(),
        1 => t!("depuis hier", "since yesterday").into(),
        n => tf!("depuis {n} jours", "for {n} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_resets_every_day() {
        let mut t = Tasks::default();
        t.add("Rendu");
        let id = t.list[0].id;
        t.set_status(id, Status::Done);
        t.roll_over(now());
        assert_eq!(t.column(Status::Done).len(), 1, "archivée aujourd'hui : reste visible");
        t.roll_over(now() + 2 * DAY_MS);
        assert!(t.column(Status::Done).is_empty());
        assert_eq!(t.archive.len(), 1);
        t.restore(id);
        assert_eq!(t.column(Status::Todo).len(), 1);
    }

    #[test]
    fn reminders_for_old_tasks() {
        let mut t = Tasks::default();
        t.add("Ancienne");
        t.add("Récente");
        t.list[0].since -= 2 * DAY_MS;
        t.add("Longue");
        let id = t.list[2].id;
        t.set_status(id, Status::Doing);
        t.list[2].since -= 4 * DAY_MS;
        let r = t.reminders(now());
        let names: Vec<&str> = r.iter().map(|(x, _)| x.title.as_str()).collect();
        assert_eq!(names, ["Longue", "Ancienne"]);
    }
}
