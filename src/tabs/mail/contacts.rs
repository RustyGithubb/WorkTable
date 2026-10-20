//! Annuaire local : chaque adresse rencontrée (expéditeurs des mails reçus, destinataires
//! des mails envoyés) est enregistrée dans `contacts.json`, avec la possibilité de la
//! mettre en favori. Sert à filtrer les mails par adresse et à l'autocomplétion.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Contact {
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub favorite: bool,
    /// Date (ms) du dernier mail échangé.
    #[serde(default)]
    pub last_seen: i64,
    /// Nombre de mails différents vus de / vers cette adresse.
    #[serde(default)]
    pub count: u32,
}

impl Contact {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.email } else { &self.name }
    }
    /// Forme « Nom <adresse> » pour un champ destinataire.
    pub fn mailbox(&self) -> String {
        if self.name.is_empty() || self.name.contains(['<', '>', '@']) {
            self.email.clone()
        } else if self.name.contains([',', ';', '"']) {
            format!("\"{}\" <{}>", self.name.replace('"', ""), self.email)
        } else {
            format!("{} <{}>", self.name, self.email)
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    contacts: Vec<Contact>,
    /// Identifiants des mails déjà comptés (évite de recompter à chaque relève).
    #[serde(default)]
    seen: Vec<String>,
}

#[derive(Default)]
pub struct Contacts {
    map: HashMap<String, Contact>,
    seen: std::collections::HashSet<String>,
    dirty: bool,
    last_save: Option<Instant>,
}

/// Sépare « Nom <adresse> » → (nom, adresse en minuscules).
pub fn parse_mailbox(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let (name, addr) = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => (s[..a].trim(), s[a + 1..b].trim()),
        _ => ("", s),
    };
    let addr = addr.trim_matches(['"', '\'', ' ']).to_lowercase();
    if !addr.contains('@') || addr.contains(char::is_whitespace) {
        return None;
    }
    let name = name.trim_matches(['"', '\'', ' ']).replace("\\\"", "\"");
    Some((name, addr))
}

/// Découpe une liste d'adresses en respectant les guillemets (« "Dupont, Jean" <j@d.fr>, a@b.fr »).
pub fn split_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' | ';' if !quoted => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("contacts.json"))
}

/// Minuscules sans accents, pour une recherche tolérante.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

impl Contacts {
    pub fn load() -> Self {
        let file: File = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Contacts {
            map: file.contacts.into_iter().map(|c| (c.email.clone(), c)).collect(),
            seen: file.seen.into_iter().collect(),
            dirty: false,
            last_save: None,
        }
    }

    pub fn save(&mut self) {
        let Some(p) = path() else { return };
        let mut contacts: Vec<Contact> = self.map.values().cloned().collect();
        contacts.sort_by(|a, b| a.email.cmp(&b.email));
        let file = File { contacts, seen: self.seen.iter().cloned().collect() };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            // Écriture atomique : fichier temporaire puis renommage.
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
        self.dirty = false;
        self.last_save = Some(Instant::now());
    }

    /// Sauvegarde différée (au plus toutes les 3 s) pour ne pas écrire à chaque mail.
    pub fn autosave(&mut self) {
        if self.dirty && self.last_save.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            self.save();
        }
    }

    /// Enregistre une adresse vue dans le mail `message_id` (daté `date`).
    pub fn record(&mut self, mailbox: &str, date: i64, message_id: &str, own: &str) {
        for part in split_list(mailbox) {
            let Some((name, email)) = parse_mailbox(&part) else { continue };
            if email == own.to_lowercase() {
                continue;
            }
            let key = format!("{message_id}/{email}");
            let new_message = !message_id.is_empty() && self.seen.insert(key);
            let c = self.map.entry(email.clone()).or_insert_with(|| Contact {
                email: email.clone(),
                name: String::new(),
                favorite: false,
                last_seen: 0,
                count: 0,
            });
            if !name.is_empty() && name != email && (c.name.is_empty() || date >= c.last_seen) && c.name != name {
                c.name = name;
                self.dirty = true;
            }
            if date > c.last_seen {
                c.last_seen = date;
                self.dirty = true;
            }
            if new_message {
                c.count += 1;
                self.dirty = true;
            }
        }
    }

    pub fn get(&self, email: &str) -> Option<&Contact> {
        self.map.get(&email.to_lowercase())
    }

    pub fn toggle_favorite(&mut self, email: &str) {
        if let Some(c) = self.map.get_mut(&email.to_lowercase()) {
            c.favorite = !c.favorite;
            self.save();
        }
    }

    pub fn remove(&mut self, email: &str) {
        if self.map.remove(&email.to_lowercase()).is_some() {
            self.save();
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Contacts correspondant à `query` (nom ou adresse), les plus pertinents d'abord :
    /// favoris, puis début de mot, puis récence.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Contact> {
        let q = fold(query.trim());
        let mut hits: Vec<(u8, &Contact)> = self
            .map
            .values()
            .filter_map(|c| {
                if q.is_empty() {
                    return Some((0, c));
                }
                let (name, email) = (fold(&c.name), c.email.as_str());
                let prefix = email.starts_with(&q)
                    || name.starts_with(&q)
                    || name.split([' ', '-', '.']).any(|w| w.starts_with(&q))
                    || email.split(['.', '@', '_', '-']).any(|w| w.starts_with(&q));
                if prefix {
                    Some((2, c))
                } else if name.contains(&q) || email.contains(&q) {
                    Some((1, c))
                } else {
                    None
                }
            })
            .collect();
        hits.sort_by(|(sa, a), (sb, b)| {
            b.favorite
                .cmp(&a.favorite)
                .then(sb.cmp(sa))
                .then(b.last_seen.cmp(&a.last_seen))
                .then(a.email.cmp(&b.email))
        });
        hits.into_iter().take(limit).map(|(_, c)| c).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        assert_eq!(parse_mailbox("Léa Martin <Lea@Studio.fr>"), Some(("Léa Martin".into(), "lea@studio.fr".into())));
        assert_eq!(parse_mailbox("\"Dupont, Jean\" <j@d.fr>"), Some(("Dupont, Jean".into(), "j@d.fr".into())));
        assert_eq!(parse_mailbox("a@b.fr"), Some((String::new(), "a@b.fr".into())));
        assert_eq!(parse_mailbox("pas une adresse"), None);
        assert_eq!(split_list("\"Dupont, Jean\" <j@d.fr>, a@b.fr; c@d.fr"), vec!["\"Dupont, Jean\" <j@d.fr>", "a@b.fr", "c@d.fr"]);
    }

    #[test]
    fn records_and_searches() {
        let mut c = Contacts::default();
        c.record("Léa Martin <lea@studio.fr>", 100, "m1", "moi@gmail.com");
        c.record("Léa Martin <lea@studio.fr>", 100, "m1", "moi@gmail.com"); // même mail : pas recompté
        c.record("Léa Martin <lea@studio.fr>", 200, "m2", "moi@gmail.com");
        c.record("Hugo Bernard <hugo@gmail.com>", 300, "m3", "moi@gmail.com");
        c.record("Moi <moi@gmail.com>", 300, "m4", "moi@gmail.com"); // soi-même : ignoré
        c.record("Steam <noreply@steampowered.com>", 50, "m5", "moi@gmail.com");
        assert_eq!(c.len(), 3);
        assert_eq!(c.get("lea@studio.fr").unwrap().count, 2);

        // Recherche sans accent, par nom ou adresse.
        assert_eq!(c.search("lea", 5)[0].email, "lea@studio.fr");
        assert_eq!(c.search("bern", 5)[0].email, "hugo@gmail.com");
        assert_eq!(c.search("gmail", 5)[0].email, "hugo@gmail.com");

        // Les favoris passent devant.
        c.map.get_mut("noreply@steampowered.com").unwrap().favorite = true;
        assert_eq!(c.search("", 5)[0].email, "noreply@steampowered.com");
        assert_eq!(c.search("", 5)[1].email, "hugo@gmail.com"); // puis le plus récent
    }

    #[test]
    fn mailbox_format() {
        let c = Contact { email: "j@d.fr".into(), name: "Dupont, Jean".into(), favorite: false, last_seen: 0, count: 0 };
        assert_eq!(c.mailbox(), "\"Dupont, Jean\" <j@d.fr>");
    }
}
