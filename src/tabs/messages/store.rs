//! Copie locale chiffrée des conversations Google Chat les plus récentes.
//!
//! - Toutes les conversations et tous les groupes (noms, membres, sections, personnes), et les
//!   [`KEEP_MSGS`] derniers messages des [`KEEP_SPACES`] conversations les plus récentes, sont
//!   gardés dans `cache\chat.wtc` : relus dès le lancement (dans un fil à part, avant même la
//!   connexion à Google), l'onglet Messages et ses conversations s'ouvrent aussitôt, même hors
//!   ligne, puis se mettent à jour depuis Google.
//! - Le fichier est chiffré (AES-256-GCM). Sa clé, tirée au hasard, est dans
//!   `secrets\chat_cache.key` ; sous Windows elle y est en plus protégée par le système (DPAPI) :
//!   seule la session Windows qui l'a créée peut la relire.
//! - Le fichier est lié au compte Google : un autre compte ne le relit pas, et il est effacé à la
//!   déconnexion (ou depuis Réglages › Données).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::google::chat::{ChatMessage, Section, Space};

/// Nombre de conversations gardées (les plus récentes).
pub const KEEP_SPACES: usize = 50;
/// Derniers messages gardés par conversation.
pub const KEEP_MSGS: usize = 200;
/// En-tête du fichier (format, version).
const MAGIC: &[u8; 4] = b"WTC1";

/// Personne (identifiant « users/… », nom, adresse, photo).
pub type PersonRow = (String, String, String, Option<String>);

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// Compte Google (adresse en minuscules) auquel appartient la copie.
    pub account: String,
    /// Date d'enregistrement (ms).
    pub saved: i64,
    pub me: Option<String>,
    /// Toutes les conversations, de la plus récente à la plus ancienne.
    pub spaces: Vec<Space>,
    /// Messages des conversations les plus récentes.
    pub msgs: Vec<(String, Vec<ChatMessage>)>,
    pub last_read: Vec<(String, i64)>,
    /// Nom affiché de chaque conversation (groupes sans nom compris : noms des membres).
    pub titles: Vec<(String, String)>,
    /// Membres de chaque conversation.
    pub members: Vec<(String, Vec<String>)>,
    /// Personnes connues (noms des membres et des auteurs).
    pub people: Vec<PersonRow>,
    /// Sections de Google Chat (rangement des groupes).
    pub sections: Vec<Section>,
}

impl Snapshot {
    /// Toutes les conversations ; messages des plus récentes seulement (sans les messages en
    /// cours d'envoi ou en échec). Noms, membres, personnes et sections sont ajoutés ensuite.
    pub fn new(account: &str, me: Option<String>, spaces: &[Space], msgs: &HashMap<String, Vec<ChatMessage>>, last_read: &HashMap<String, i64>) -> Self {
        let all = recent_spaces(spaces, usize::MAX);
        let keep = |name: &str| all.iter().take(KEEP_SPACES).any(|s| s.name == name);
        let msgs = msgs
            .iter()
            .filter(|(sp, _)| keep(sp))
            .map(|(sp, list)| {
                let list: Vec<ChatMessage> = list.iter().filter(|m| !m.name.contains("local/") && !m.name.starts_with("failed/")).cloned().collect();
                let skip = list.len().saturating_sub(KEEP_MSGS);
                (sp.clone(), list.into_iter().skip(skip).collect())
            })
            .collect();
        let last_read = last_read.iter().map(|(k, v)| (k.clone(), *v)).collect();
        Snapshot { account: account.to_lowercase(), saved: chrono::Utc::now().timestamp_millis(), me, spaces: all, msgs, last_read, ..Default::default() }
    }
}

/// Les `n` conversations à l'activité la plus récente.
pub fn recent_spaces(spaces: &[Space], n: usize) -> Vec<Space> {
    let mut list = spaces.to_vec();
    list.sort_by_key(|s| std::cmp::Reverse(s.last_active));
    list.truncate(n);
    list
}

fn file() -> PathBuf {
    crate::paths::cache().join("chat.wtc")
}

fn key_file() -> PathBuf {
    crate::paths::secrets().join("chat_cache.key")
}

/// Clé de chiffrement (créée au premier enregistrement si `create`).
fn key(create: bool) -> Option<[u8; 32]> {
    if let Ok(raw) = std::fs::read(key_file())
        && let Some(k) = unprotect(&raw).and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
    {
        return Some(k);
    }
    if !create {
        return None;
    }
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).ok()?;
    crate::paths::write_atomic(&key_file(), &protect(&k)?).ok()?;
    Some(k)
}

/// Chiffre et enregistre la copie (dans un fil à part : rien ne bloque l'interface).
/// `then_remove` : ancien fichier en clair, supprimé une fois la copie chiffrée écrite.
pub fn save(snap: Snapshot, then_remove: Option<PathBuf>) {
    std::thread::spawn(move || {
        if write(&snap).is_ok()
            && let Some(p) = then_remove
        {
            let _ = std::fs::remove_file(p);
        }
    });
}

/// Enregistrement immédiat (fermeture de l'app).
pub fn save_now(snap: &Snapshot) {
    let _ = write(snap);
}

fn write(snap: &Snapshot) -> Result<(), String> {
    let plain = serde_json::to_vec(snap).map_err(|e| e.to_string())?;
    write_sealed(&file(), MAGIC, &plain)
}

/// Chiffre `plain` avec la clé de la copie locale et l'écrit dans `path` (en-tête `magic`).
/// Sert aussi à l'annuaire caché des fiches contact.
pub(crate) fn write_sealed(path: &std::path::Path, magic: &[u8; 4], plain: &[u8]) -> Result<(), String> {
    let k = key(true).ok_or("clé indisponible")?;
    let (nonce, ct) = crate::notes_store::encrypt(&k, plain)?;
    let mut out = Vec::with_capacity(magic.len() + nonce.len() + ct.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    crate::paths::write_atomic(path, &out).map_err(|e| e.to_string())
}

/// Relit et déchiffre un fichier écrit par [`write_sealed`].
pub(crate) fn read_sealed(path: &std::path::Path, magic: &[u8; 4]) -> Option<Vec<u8>> {
    let raw = std::fs::read(path).ok()?;
    let rest = raw.strip_prefix(magic)?;
    if rest.len() < 12 {
        return None;
    }
    crate::notes_store::decrypt(&key(false)?, &rest[..12], &rest[12..]).ok()
}

/// Relit la copie dans un fil à part (déchiffrement et lecture ne retardent pas l'affichage) ;
/// le résultat arrive par le canal (`None` si elle manque ou ne peut pas être déchiffrée).
/// Le compte est vérifié par l'appelant dès qu'il est connu.
pub fn load_async() -> std::sync::mpsc::Receiver<Option<Snapshot>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(load());
    });
    rx
}

fn load() -> Option<Snapshot> {
    serde_json::from_slice(&read_sealed(&file(), MAGIC)?).ok()
}

/// Efface la copie locale (conversations et fiches contact) et sa clé.
pub fn clear() {
    let _ = std::fs::remove_file(file());
    let _ = std::fs::remove_file(crate::people::file());
    let _ = std::fs::remove_file(key_file());
}

/// Taille du fichier enregistré (octets), s'il existe.
pub fn size() -> Option<u64> {
    std::fs::metadata(file()).ok().map(|m| m.len())
}

// --- Protection de la clé par Windows (DPAPI) ----------------------------------------

#[cfg(windows)]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, true)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    dpapi(data, false)
}

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY : tampons valides le temps de l'appel ; la sortie, allouée par Windows, est copiée
    // puis libérée par LocalFree.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Some(out)
    }
}

/// Ailleurs : la clé est gardée telle quelle dans le dossier `secrets` (jamais publié).
#[cfg(not(windows))]
fn protect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::chat::SpaceKind;

    fn space(i: i64) -> Space {
        Space { name: format!("spaces/{i}"), display_name: String::new(), kind: SpaceKind::Dm, threaded: false, last_active: i, members: 2, bot_dm: false }
    }

    #[test]
    fn keeps_recent_spaces_and_messages() {
        let spaces: Vec<Space> = (0..70).map(space).collect();
        let mut msgs = HashMap::new();
        let list: Vec<ChatMessage> = (0..250).map(|i| ChatMessage { name: format!("spaces/69/messages/{i}"), created: i, ..Default::default() }).collect();
        msgs.insert("spaces/69".to_string(), list);
        msgs.insert("spaces/0".to_string(), vec![ChatMessage::default()]);
        let snap = Snapshot::new("Moi@X.fr", None, &spaces, &msgs, &HashMap::new());
        assert_eq!(snap.account, "moi@x.fr");
        assert_eq!(snap.spaces.len(), 70, "toutes les conversations (pour leurs noms)");
        assert_eq!(snap.spaces[0].name, "spaces/69", "la plus récente en tête");
        assert_eq!(snap.msgs.len(), 1, "conversation trop ancienne écartée");
        assert_eq!(snap.msgs[0].1.len(), KEEP_MSGS);
        assert_eq!(snap.msgs[0].1.last().unwrap().created, 249, "les derniers messages sont gardés");
    }

    #[test]
    fn encrypted_round_trip() {
        let k = [7u8; 32];
        let mut snap = Snapshot::new("a@b.c", Some("users/1".into()), &[space(1)], &HashMap::new(), &HashMap::new());
        snap.titles.push(("spaces/1".into(), "Projet Weeping Desert".into()));
        let plain = serde_json::to_vec(&snap).unwrap();
        let (nonce, ct) = crate::notes_store::encrypt(&k, &plain).unwrap();
        assert!(!ct.windows(5).any(|w| w == b"a@b.c"), "contenu illisible sans la clé");
        let back: Snapshot = serde_json::from_slice(&crate::notes_store::decrypt(&k, &nonce, &ct).unwrap()).unwrap();
        assert_eq!(back.spaces[0].name, "spaces/1");
        assert_eq!(back.me.as_deref(), Some("users/1"));
        assert_eq!(back.titles[0].1, "Projet Weeping Desert");
        // Copie d'une version précédente (sans noms) : relue quand même.
        let old: Snapshot = serde_json::from_str(r#"{"account":"a@b.c","saved":0,"me":null,"spaces":[],"msgs":[],"last_read":[]}"#).unwrap();
        assert!(old.titles.is_empty() && old.people.is_empty());
    }

    /// Écrit puis relit le vrai fichier. Ignoré par défaut (il écrit dans le dossier des données) :
    /// `WORKTABLE_DATA=<dossier temporaire> cargo test -- --ignored store_file_round_trip`.
    #[test]
    #[ignore]
    fn store_file_round_trip() {
        assert!(std::env::var_os("WORKTABLE_DATA").is_some(), "à lancer avec WORKTABLE_DATA");
        let mut snap = Snapshot::new("a@b.c", None, &[space(5)], &HashMap::new(), &HashMap::new());
        snap.titles.push(("spaces/5".into(), "Équipe".into()));
        snap.people.push(("users/1".into(), "Léa".into(), "lea@b.c".into(), None));
        save_now(&snap);
        let raw = std::fs::read(file()).unwrap();
        assert!(raw.starts_with(MAGIC) && !raw.windows(4).any(|w| w == "Léa".as_bytes()), "fichier chiffré");
        let back = load_async().recv().unwrap().expect("relue");
        assert_eq!(back.titles[0].1, "Équipe");
        assert_eq!(back.people[0].1, "Léa");
        clear();
        assert!(load_async().recv().unwrap().is_none());
    }
}
