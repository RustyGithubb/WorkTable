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
            Curve::Linear => t!("Linéaire", "Linear"),
            Curve::EaseIn => "Ease in",
            Curve::EaseOut => "Ease out",
            Curve::EaseInOut => "Ease in-out",
            Curve::Elastic => t!("Élastique", "Elastic"),
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

/// Réglages de l'onglet Blueprint.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BlueprintPrefs {
    /// Opacité de la grille du graph (0 = invisible, 1 = normale).
    pub grid_opacity: f32,
}

impl Default for BlueprintPrefs {
    fn default() -> Self {
        BlueprintPrefs { grid_opacity: 1.0 }
    }
}

/// Rangée du tableau de bord : hauteur (en crans de la grille) et panneaux, de gauche à droite,
/// chacun avec sa part de la largeur (la somme fait toujours 1, soit 100 %).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DashRow {
    pub h: u8,
    pub panels: Vec<DashPanel>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DashPanel {
    pub id: String,
    /// Part de la largeur de la rangée (0 à 1).
    pub share: f32,
}

/// Quadrillage du tableau de bord : un carré par jour, du premier au dernier jour réglés
/// (vides : l'année scolaire française en cours).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TimelinePrefs {
    /// Titre du panneau (vide : « Année scolaire »).
    pub title: String,
    /// Premier et dernier jour, « AAAA-MM-JJ ».
    pub start: String,
    pub end: String,
    /// Événements du calendrier mis en évidence (leur jour est coloré).
    pub highlights: Vec<Highlight>,
    /// Périodes de vacances déclarées (affichées comme les week-ends dans le quadrillage).
    pub holidays: Vec<Holiday>,
}

/// Période de vacances : du `start` au `end` inclus (« AAAA-MM-JJ »), avec un nom facultatif.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Holiday {
    pub start: String,
    pub end: String,
    pub name: String,
}

/// Événement mis en évidence dans le quadrillage (copie tenue à jour quand l'événement est chargé).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Highlight {
    pub calendar: String,
    pub id: String,
    pub title: String,
    /// Début et fin (ms ; fin exclue).
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

/// Autre base de jours mis en évidence (la base d'origine est `TimelinePrefs::highlights`) :
/// utilisée par les quadrillages des onglets personnalisés.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HighlightSet {
    pub id: u64,
    pub name: String,
    pub highlights: Vec<Highlight>,
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
    /// Couleur de peau choisie pour les émojis (0 = jaune par défaut, 1 à 5 = claire → foncée).
    pub emoji_tone: u8,
    /// Navigation de l'onglet Fichiers masquée.
    pub files_side_hidden: bool,
    /// Liste des graphs de l'onglet Blueprint masquée.
    pub blueprint_side_hidden: bool,
    /// Réglages de l'onglet Blueprint (Réglages › Blueprint).
    pub blueprint: BlueprintPrefs,
    /// Quadrillage des jours du tableau de bord (Réglages › Quadrillage).
    pub timeline: TimelinePrefs,
    /// Autres bases de jours mis en évidence (choisies dans le Calendrier).
    pub highlight_sets: Vec<HighlightSet>,
    /// Disposition du tableau de bord (rangées, ordre et parts des panneaux) ; vide : par défaut.
    pub dashboard_rows: Vec<DashRow>,
    /// Nombre de fichiers du panneau « Fichiers reçus » (0 : par défaut).
    pub shared_files: u8,
    /// Polices et tailles par groupe (titres, interface, texte, code).
    pub fonts: crate::theme::FontPrefs,
    /// Thème de couleurs (Réglages › Apparence).
    pub theme: crate::theme::ThemeId,
    /// Notifications désactivées par type (toutes actives par défaut), avance des rappels (min, 0 = 10).
    pub notif_no_mail: bool,
    pub notif_no_chat: bool,
    pub notif_no_share: bool,
    pub notif_no_event: bool,
    /// Pas de son à l'arrivée d'un mail ou d'un message.
    pub notif_no_sound: bool,
    /// « Ne pas déranger » (bouton du tableau de bord) : aucun bandeau ni son jusqu'à ce qu'on
    /// le désactive.
    pub do_not_disturb: bool,
    /// Heure en haut à droite masquée (clic dessus ; réaffichable dans Réglages › Apparence).
    pub clock_hidden: bool,
    pub notif_event_minutes: u32,
    /// Langue de l'interface (celle du système par défaut).
    pub language: crate::i18n::Language,
    /// Raccourci global qui rouvre l'app restée en arrière-plan (Ctrl+Maj+Espace par défaut).
    pub show_hotkey: Hotkey,
    /// Raccourci global « quitter WorkTable » ; absent : Ctrl+Maj+K.
    pub quit_hotkey: Option<Hotkey>,
    /// Correcteur d'orthographe désactivé, langues retirées (toutes actives par défaut).
    pub spell_off: bool,
    pub spell_no_fr: bool,
    pub spell_no_en: bool,
    /// Clé de l'API GIPHY (sélecteur de GIF des messages) et GIF favoris.
    pub giphy_key: String,
    pub chat_gif_favorites: Vec<GifItem>,
    /// Onglets personnalisés (galerie 3 × 3 du logo de la barre) : un fichier par onglet dans
    /// `data\custom_tabs` (voir [`custom_tabs`]) ; lus dans `prefs.json` seulement s'ils y sont
    /// encore (anciennes versions), puis déplacés.
    #[serde(skip_serializing)]
    pub custom_tabs: Vec<CustomTab>,
}

/// Onglet personnalisé : case de la galerie (0 à 8), nom, logo (émoji, ou image copiée dans
/// `config\custom_tabs`).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomTab {
    pub id: u64,
    pub slot: u8,
    pub name: String,
    pub emoji: String,
    pub image: Option<String>,
    /// Panneaux de l'onglet (même grille que le tableau de bord) ; vide : aucun panneau.
    pub rows: Vec<DashRow>,
    /// Texte des panneaux « Bloc-notes », par panneau.
    pub notes: std::collections::BTreeMap<String, String>,
    /// Base de jours mis en évidence de chaque quadrillage (panneau → `HighlightSet::id`).
    pub grid_sets: std::collections::BTreeMap<String, u64>,
}

/// GIF du sélecteur (GIPHY) : aperçu léger pour la grille, version envoyée.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GifItem {
    pub id: String,
    pub title: String,
    /// Petit GIF animé (grille).
    pub preview: String,
    /// GIF envoyé (≤ 2 Mo si possible).
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// Raccourci clavier global : modificateurs et touche (nom egui, ex. « Space », « K »).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Vide : aucun raccourci.
    pub key: String,
}

impl Prefs {
    /// Raccourci « quitter » (réglé, sinon Ctrl+Maj+K).
    pub fn quit_key(&self) -> Hotkey {
        self.quit_hotkey.clone().unwrap_or_else(crate::background::default_quit_key)
    }
}

impl Default for Hotkey {
    fn default() -> Self {
        Self { ctrl: true, shift: true, alt: false, key: "Space".into() }
    }
}

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("prefs.json"))
}

impl Prefs {
    pub fn load() -> Self {
        let mut prefs: Prefs = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let old = std::mem::take(&mut prefs.custom_tabs);
        prefs.custom_tabs = custom_tabs::load(old);
        prefs
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            crate::paths::save_json(&p, self);
        }
        custom_tabs::save(&self.custom_tabs);
    }
}

/// Onglets personnalisés : un fichier JSON par onglet (`data\custom_tabs\<id>.json`), à
/// partager tel quel. Un fichier déposé dans ce dossier est importé au lancement suivant (dans
/// une case libre de la galerie).
pub mod custom_tabs {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::CustomTab;

    /// Onglets lus ou écrits par l'app : leur fichier est supprimé quand l'onglet l'est (un
    /// fichier déposé pendant que l'app tourne n'est pas touché).
    static KNOWN: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

    pub fn dir() -> PathBuf {
        crate::paths::data().join("custom_tabs")
    }

    fn file(id: u64) -> PathBuf {
        dir().join(format!("{id}.json"))
    }

    /// Onglets du dossier ; `old` : ceux encore dans `prefs.json` (anciennes versions), gardés
    /// s'ils n'ont pas de fichier.
    pub fn load(old: Vec<CustomTab>) -> Vec<CustomTab> {
        let mut tabs: Vec<CustomTab> = std::fs::read_dir(dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str::<CustomTab>(&std::fs::read_to_string(e.path()).ok()?).ok())
            .collect();
        for t in old {
            if !tabs.iter().any(|x| x.id == t.id) {
                tabs.push(t);
            }
        }
        let known: HashSet<u64> = tabs.iter().map(|t| t.id).collect();
        let tabs = place(tabs);
        *KNOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(known);
        tabs
    }

    /// Une case par onglet : un onglet importé dont la case est prise (ou invalide) va dans la
    /// première case libre ; sans case libre, il reste dans son fichier.
    fn place(mut tabs: Vec<CustomTab>) -> Vec<CustomTab> {
        tabs.sort_by_key(|t| (t.slot, t.id));
        let mut used = [false; crate::gallery::SLOTS];
        let mut out = Vec::new();
        for mut t in tabs {
            let slot = t.slot as usize;
            let free = if slot < used.len() && !used[slot] { Some(slot) } else { used.iter().position(|u| !u) };
            if let Some(s) = free {
                used[s] = true;
                t.slot = s as u8;
                out.push(t);
            }
        }
        out
    }

    /// Écrit chaque onglet dans son fichier (s'il a changé) et retire les fichiers des onglets
    /// supprimés.
    pub fn save(tabs: &[CustomTab]) {
        let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
        let known = known.get_or_insert_with(HashSet::new);
        for t in tabs {
            let path = file(t.id);
            let Ok(text) = serde_json::to_string_pretty(t) else { continue };
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                let _ = crate::paths::write_atomic(&path, text.as_bytes());
            }
            known.insert(t.id);
        }
        known.retain(|id| {
            let keep = tabs.iter().any(|t| t.id == *id);
            if !keep {
                let _ = std::fs::remove_file(file(*id));
            }
            keep
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn imported_tabs_get_a_free_slot() {
            let t = |id: u64, slot: u8| CustomTab { id, slot, ..Default::default() };
            let out = place(vec![t(1, 0), t(2, 0), t(3, 40)]);
            assert_eq!(out.iter().map(|t| (t.id, t.slot)).collect::<Vec<_>>(), [(1, 0), (2, 1), (3, 2)]);
            let full: Vec<CustomTab> = (0..10).map(|i| t(i, 0)).collect();
            assert_eq!(place(full).len(), crate::gallery::SLOTS);
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
