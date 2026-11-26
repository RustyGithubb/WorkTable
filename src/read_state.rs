//! États de lecture partagés entre les postes du compte, par la synchronisation du NAS :
//! `data\read_state.json` (chiffré de bout en bout comme le reste de `data`).
//!
//! Chat : date de dernière lecture de chaque conversation (ms). Google garde aussi cette date ;
//! cette copie sert dès le démarrage, avant sa réponse, et hors ligne. Les mails n'en ont pas
//! besoin : « lu » est une étiquette Gmail, commune à tous les appareils.
//!
//! Chaque poste n'écrit que sous son nom : deux postes ne modifient jamais la même valeur, donc
//! la fusion de la synchro (clé par clé) n'a jamais de conflit. À la lecture, la date la plus
//! récente de tous les postes l'emporte.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct ReadState {
    /// Poste → conversation (« spaces/… ») → dernière lecture (ms).
    chat: BTreeMap<String, BTreeMap<String, i64>>,
}

fn path() -> PathBuf {
    crate::paths::data().join("read_state.json")
}

fn load() -> ReadState {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Dernières lectures des conversations, tous postes confondus (la plus récente).
pub fn load_chat() -> HashMap<String, i64> {
    let mut out: HashMap<String, i64> = HashMap::new();
    for (sp, t) in load().chat.into_values().flatten() {
        let e = out.entry(sp).or_insert(t);
        *e = (*e).max(t);
    }
    out
}

/// Enregistre les dernières lectures connues de ce poste (sous son nom ; la plus récente gagne).
/// Le fichier n'est réécrit que s'il change, pour ne pas renvoyer au NAS une copie identique.
pub fn save_chat(last_read: &HashMap<String, i64>) {
    let old = load();
    let mut new = ReadState { chat: old.chat.clone() };
    let mine = new.chat.entry(crate::sync::device_name()).or_default();
    for (sp, t) in last_read {
        let e = mine.entry(sp.clone()).or_insert(*t);
        *e = (*e).max(*t);
    }
    if new != old {
        crate::paths::save_json(&path(), &new);
    }
}
