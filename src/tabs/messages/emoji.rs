//! Émojis : catalogue Unicode complet (crate `emojis`), sélecteur par catégories avec recherche
//! (noms anglais, codes « :smile: » et quelques mots français), codes courts « :nom: »
//! remplacés pendant la frappe comme dans Google Chat.

use std::sync::Arc;

use eframe::egui::{self, RichText, vec2};

use crate::theme;

/// Catégories (icône d'onglet, nom français, nom anglais).
const GROUPS: [(emojis::Group, &str, &str, &str); 9] = [
    (emojis::Group::SmileysAndEmotion, "😀", "Émoticônes", "Smileys"),
    (emojis::Group::PeopleAndBody, "👋", "Personnes", "People"),
    (emojis::Group::AnimalsAndNature, "🐻", "Animaux et nature", "Animals & nature"),
    (emojis::Group::FoodAndDrink, "🍔", "Nourriture", "Food & drink"),
    (emojis::Group::TravelAndPlaces, "🚗", "Voyages et lieux", "Travel & places"),
    (emojis::Group::Activities, "⚽", "Activités", "Activities"),
    (emojis::Group::Objects, "💡", "Objets", "Objects"),
    (emojis::Group::Symbols, "❤️", "Symboles", "Symbols"),
    (emojis::Group::Flags, "🏁", "Drapeaux", "Flags"),
];

/// Quelques mots français → mots-clés anglais du catalogue.
const FR: [(&str, &str); 40] = [
    ("coeur", "heart"),
    ("cœur", "heart"),
    ("rire", "laugh"),
    ("mdr", "joy"),
    ("sourire", "smil"),
    ("pouce", "thumbs"),
    ("ok", "ok"),
    ("feu", "fire"),
    ("fete", "party"),
    ("fête", "party"),
    ("merci", "folded hands"),
    ("bravo", "clapping"),
    ("triste", "sad"),
    ("pleur", "cry"),
    ("colere", "angry"),
    ("colère", "angry"),
    ("chat", "cat"),
    ("chien", "dog"),
    ("soleil", "sun"),
    ("pluie", "rain"),
    ("cafe", "coffee"),
    ("café", "coffee"),
    ("biere", "beer"),
    ("bière", "beer"),
    ("gateau", "cake"),
    ("gâteau", "cake"),
    ("fusee", "rocket"),
    ("fusée", "rocket"),
    ("yeux", "eyes"),
    ("etoile", "star"),
    ("étoile", "star"),
    ("cadeau", "gift"),
    ("ordinateur", "computer"),
    ("telephone", "phone"),
    ("argent", "money"),
    ("check", "check"),
    ("croix", "cross"),
    ("main", "hand"),
    ("musique", "music"),
    ("jeu", "game"),
];

/// Émojis affichables par la police de l'application (sans séquences composées ni teintes de peau).
pub fn catalog(ctx: &egui::Context) -> Arc<Vec<&'static emojis::Emoji>> {
    let id = egui::Id::unique("wt_emoji_catalog");
    if let Some(c) = ctx.data(|d| d.get_temp::<Arc<Vec<&'static emojis::Emoji>>>(id)) {
        return c;
    }
    // Images couleur (voir `emoji_img`) : tout le catalogue, sans les variantes de teinte de peau.
    let list: Vec<&'static emojis::Emoji> = emojis::iter().filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default)).collect();
    let list = Arc::new(list);
    ctx.data_mut(|d| d.insert_temp(id, list.clone()));
    list
}

fn matches(e: &emojis::Emoji, q: &str) -> bool {
    let name = e.name();
    if name.contains(q) || e.shortcodes().any(|s| s.contains(q)) {
        return true;
    }
    FR.iter().any(|(fr, en)| fr.starts_with(q) && q.len() >= 2 && (name.contains(en) || e.shortcodes().any(|s| s.contains(en))))
}

/// Suggestions pour un code en cours de frappe (« :smi »).
pub fn suggestions(ctx: &egui::Context, prefix: &str, n: usize) -> Vec<&'static emojis::Emoji> {
    let q = prefix.to_lowercase();
    let cat = catalog(ctx);
    let mut out: Vec<&'static emojis::Emoji> = cat.iter().copied().filter(|e| e.shortcodes().any(|s| s.starts_with(&q))).take(n).collect();
    if out.len() < n {
        for e in cat.iter().copied().filter(|e| matches(e, &q)) {
            if out.len() >= n {
                break;
            }
            if !out.iter().any(|x| x.as_str() == e.as_str()) {
                out.push(e);
            }
        }
    }
    out
}

/// Émojis utilisés récemment, partagés par tous les onglets (enregistrés dans les réglages).
fn recent_list() -> &'static std::sync::Mutex<Vec<String>> {
    static R: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

pub fn recent() -> Vec<String> {
    recent_list().lock().map(|l| l.clone()).unwrap_or_default()
}

pub fn set_recent(list: Vec<String>) {
    if let Ok(mut l) = recent_list().lock() {
        *l = list;
    }
}

pub fn remember(e: &str) {
    if let Ok(mut l) = recent_list().lock() {
        l.retain(|x| x != e);
        l.insert(0, e.to_string());
        l.truncate(32);
    }
}

/// Code « :nom » juste avant le curseur : (position du « : » en caractères, préfixe).
pub fn pending_code_at(text: &str, cursor: usize) -> Option<(usize, String)> {
    let before: String = text.chars().take(cursor).collect();
    let (i, p) = pending_code(&before)?;
    Some((before[..i].chars().count(), p))
}

/// Remplace les codes « :nom: » placés avant le curseur ; renvoie le texte et le nouveau curseur.
pub fn replace_codes_at(text: &str, cursor: usize) -> Option<(String, usize)> {
    let split = text.char_indices().nth(cursor).map(|(b, _)| b).unwrap_or(text.len());
    let head = replace_codes(&text[..split])?;
    let c = head.chars().count();
    Some((head + &text[split..], c))
}

/// Code « :nom » en cours de frappe juste avant la fin du texte : (position du « : », préfixe).
pub fn pending_code(text: &str) -> Option<(usize, String)> {
    let i = text.rfind(':')?;
    let tail = &text[i + 1..];
    let before_ok = i == 0 || text[..i].ends_with(char::is_whitespace);
    (before_ok && tail.len() >= 2 && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-')).then(|| (i, tail.to_string()))
}

/// Remplace les codes complets « :nom: » par l'émoji correspondant (comme Google Chat).
pub fn replace_codes(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut rest = text;
    while let Some(a) = rest.find(':') {
        let after = &rest[a + 1..];
        match after.find(':') {
            Some(b) if b >= 1 && after[..b].chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-') => {
                if let Some(e) = emojis::get_by_shortcode(&after[..b]) {
                    out.push_str(&rest[..a]);
                    out.push_str(e.as_str());
                    rest = &after[b + 1..];
                    changed = true;
                    continue;
                }
                out.push_str(&rest[..a + 1]);
                rest = after;
            }
            _ => {
                out.push_str(&rest[..a + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// Sélecteur : récents, recherche, onglets de catégories, grille. Renvoie l'émoji choisi.
pub fn picker(ui: &mut egui::Ui, query: &mut String, group: &mut usize, recent: &[String]) -> Option<String> {
    let mut picked = None;
    ui.set_width(330.0);
    ui.add(egui::TextEdit::singleline(query).hint_text(t!("Rechercher un émoji (ex. cœur, rire, fire)", "Search emoji (e.g. heart, laugh, fire)")).desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let cat = catalog(ui.ctx());
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tab = |ui: &mut egui::Ui, icon: &str, tip: &str, on: bool| {
                let r = crate::emoji_img::button(ui, icon, 30.0);
                if on {
                    ui.painter().hline(r.rect.x_range().shrink(6.0), r.rect.bottom(), egui::Stroke::new(2.0, theme::TODAY()));
                }
                r.on_hover_text(tip).clicked()
            };
            if !recent.is_empty() && tab(ui, "🕘", t!("Récents", "Recent"), *group == usize::MAX) {
                *group = usize::MAX;
            }
            for (i, (_, icon, fr, en)) in GROUPS.iter().enumerate() {
                if tab(ui, icon, t!(fr, en), *group == i) {
                    *group = i;
                }
            }
        });
        let title = if *group == usize::MAX { t!("Récents", "Recent") } else { GROUPS.get(*group).map(|g| t!(g.2, g.3)).unwrap_or("") };
        ui.label(RichText::new(title.to_uppercase()).small().color(theme::TEXT_MUTED()));
    }
    let list: Vec<String> = if !q.is_empty() {
        cat.iter().filter(|e| matches(e, &q)).map(|e| e.as_str().to_string()).collect()
    } else if *group == usize::MAX {
        recent.to_vec()
    } else {
        let g = GROUPS.get(*group).map(|g| g.0).unwrap_or(emojis::Group::SmileysAndEmotion);
        cat.iter().filter(|e| e.group() == g).map(|e| e.as_str().to_string()).collect()
    };
    egui::ScrollArea::vertical().id_salt("wt_emoji_grid").max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
        if list.is_empty() {
            ui.label(RichText::new(t!("Aucun émoji", "No emoji")).small().color(theme::TEXT_FAINT()));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for e in &list {
                let tip = emojis::get(e).map(|x| x.shortcode().map(|s| format!(":{s}:")).unwrap_or_else(|| x.name().to_string())).unwrap_or_default();
                if crate::emoji_img::button(ui, e, 34.0).on_hover_text(tip).clicked() {
                    picked = Some(e.clone());
                }
            }
        });
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcodes_are_replaced() {
        assert_eq!(replace_codes("Salut :smile: !").as_deref(), Some("Salut 😄 !"));
        assert_eq!(replace_codes("10:30 et 11:45"), None);
        assert_eq!(pending_code("ok :thu").map(|x| x.1).as_deref(), Some("thu"));
        assert!(pending_code("il est 10:30").is_none());
    }
}
