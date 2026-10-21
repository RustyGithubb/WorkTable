//! Préférences de l'application (prefs.json dans le dossier de configuration).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, Default)]
pub enum Curve {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
    /// Léger dépassement puis retour (effet « goutte »).
    Elastic,
}

impl Curve {
    pub const ALL: [Curve; 5] = [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut, Curve::Elastic];

    pub fn label(self) -> &'static str {
        match self {
            Curve::Linear => "Linéaire",
            Curve::EaseIn => "Ease in",
            Curve::EaseOut => "Ease out",
            Curve::EaseInOut => "Ease in-out",
            Curve::Elastic => "Élastique",
        }
    }

    /// Progression `t` ∈ [0, 1] → valeur animée.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::EaseIn => t * t * t,
            Curve::EaseOut => 1.0 - (1.0 - t).powi(3),
            Curve::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Curve::Elastic => {
                // « back out » : dépasse légèrement la cible puis revient.
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Animation {
    /// Durée d'un déplacement, en millisecondes.
    pub duration_ms: f32,
    /// Opacité du surlignage liquide (0 à 1).
    pub opacity: f32,
    pub curve: Curve,
}

impl Default for Animation {
    fn default() -> Self {
        Animation { duration_ms: 450.0, opacity: 1.0, curve: Curve::EaseInOut }
    }
}

/// Couleur associée à une adresse intermédiaire (liste de diffusion, adresse de groupe).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GroupColor {
    pub address: String,
    pub color: [u8; 3],
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prefs {
    /// Adresses de groupe colorées (ex. theotter@artfx.fr).
    pub groups: Vec<GroupColor>,
    pub animation: Animation,
    /// Section « Catégories » repliée dans l'onglet Mail.
    pub categories_collapsed: bool,
    /// Liste des mails masquée (plus de place pour la lecture).
    pub mail_list_hidden: bool,
    /// Section « Groupes » repliée dans l'onglet Mail.
    pub groups_collapsed: bool,
    /// Agendas masqués dans l'onglet Calendrier (identifiants Google).
    pub hidden_calendars: Vec<String>,
    /// Agendas déjà vus (pour reprendre une seule fois l'état « affiché » de Google Agenda).
    pub known_calendars: Vec<String>,
    /// Vue du calendrier : nombre de jours affichés (1 à 7), 8 = mois, 0 = par défaut (semaine).
    pub calendar_days: u8,
    /// Colonne de gauche du calendrier masquée (agenda en pleine largeur).
    pub calendar_side_hidden: bool,
    /// Conversations Google Chat mises en favoris.
    pub chat_favorites: Vec<String>,
    /// Colonne des conversations masquée dans l'onglet Messages.
    pub chat_side_hidden: bool,
    /// Colonne « Personnes » (messages privés) masquée.
    pub chat_people_hidden: bool,
    /// Conversations rendues muettes (pas de pastille de non lus).
    pub chat_muted: Vec<String>,
    /// Émojis utilisés récemment (sélecteur).
    pub chat_recent_emojis: Vec<String>,
    /// Navigation de l'onglet Fichiers masquée.
    pub files_side_hidden: bool,
    /// Polices et tailles par groupe (titres, interface, texte, code).
    pub fonts: crate::theme::FontPrefs,
    /// Thème de couleurs (Réglages › Apparence).
    pub theme: crate::theme::ThemeId,
    /// Notifications désactivées par type (toutes actives par défaut), avance des rappels (min, 0 = 10).
    pub notif_no_mail: bool,
    pub notif_no_chat: bool,
    pub notif_no_share: bool,
    pub notif_no_event: bool,
    pub notif_no_system: bool,
    /// Pas de son à l'arrivée d'un mail ou d'un message.
    pub notif_no_sound: bool,
    pub notif_event_minutes: u32,
}

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("prefs.json"))
}

impl Prefs {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_and_end() {
        for c in Curve::ALL {
            assert!(c.apply(0.0).abs() < 1e-4, "{c:?}");
            assert!((c.apply(1.0) - 1.0).abs() < 1e-4, "{c:?}");
        }
        assert!((Curve::EaseInOut.apply(0.5) - 0.5).abs() < 1e-4);
        assert!(Curve::Elastic.apply(0.7) > 1.0, "dépassement attendu");
    }
}
