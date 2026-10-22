//! Messages Google Chat épinglés : affichés dans le tableau de bord jusqu'à ce qu'on les
//! désépingle. Copie locale du message (chat_pinned.json), consultable même hors ligne.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::google::chat::ChatMessage;

/// Nombre maximal de messages épinglés (les plus anciens laissent la place).
pub const MAX: usize = 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinnedMsg {
    /// « spaces/X/messages/Y »
    pub name: String,
    pub space: String,
    /// Nom de la conversation au moment de l'épinglage.
    pub space_title: String,
    pub sender: String,
    pub text: String,
    /// Date du message (ms).
    pub created: i64,
}

#[derive(Default)]
pub struct Pins {
    pub list: Vec<PinnedMsg>,
}

fn path() -> PathBuf {
    crate::paths::config().join("chat_pinned.json")
}

impl Pins {
    pub fn load() -> Self {
        let list = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Pins { list }
    }

    fn save(&self) {
        crate::paths::save_json(&path(), &self.list);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.list.iter().any(|p| p.name == name)
    }

    /// Épingle (en tête de liste) ou désépingle ; renvoie `true` si le message est maintenant épinglé.
    pub fn toggle(&mut self, m: &ChatMessage, sender: String, space_title: String) -> bool {
        let pinned = self.flip(m, sender, space_title);
        self.save();
        pinned
    }

    fn flip(&mut self, m: &ChatMessage, sender: String, space_title: String) -> bool {
        let pinned = !self.contains(&m.name);
        if pinned {
            let text = if m.text.trim().is_empty() && !m.attachments.is_empty() { t!("📎 Pièce jointe", "📎 Attachment").to_string() } else { m.text.clone() };
            self.list.insert(0, PinnedMsg { name: m.name.clone(), space: m.space.clone(), space_title, sender, text, created: m.created });
            self.list.truncate(MAX);
        } else {
            self.list.retain(|p| p.name != m.name);
        }
        pinned
    }

    pub fn remove(&mut self, name: &str) {
        if self.contains(name) {
            self.list.retain(|p| p.name != name);
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_and_limits() {
        let mut p = Pins::default();
        let m = |i: usize| ChatMessage { name: format!("spaces/a/messages/{i}"), space: "spaces/a".into(), text: format!("m{i}"), ..Default::default() };
        assert!(p.flip(&m(0), "Léa".into(), "Projet".into()));
        assert!(p.contains("spaces/a/messages/0"));
        assert!(!p.flip(&m(0), String::new(), String::new()));
        assert!(p.list.is_empty());
        for i in 0..MAX + 3 {
            p.flip(&m(i), String::new(), String::new());
        }
        assert_eq!(p.list.len(), MAX);
        assert_eq!(p.list[0].name, format!("spaces/a/messages/{}", MAX + 2), "le plus récent en tête");
        assert!(!p.contains("spaces/a/messages/0"), "le plus ancien laisse la place");
    }
}
