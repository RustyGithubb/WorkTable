//! Échanges avec le serveur : recevoir, appliquer (au démarrage), envoyer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nas_proto::sync::DocMeta;
use nas_sync_client::crypto::sha256_hex;
use nas_sync_client::{AppKey, Client, Error, MasterKey};
use serde::{Deserialize, Serialize};

use super::{Account, Phase, Secret, dir, merge, with_status};

/// Taille maximale d'un fichier synchronisé (le serveur refuse au-delà de 8 Mo chiffrés).
const MAX_FILE: u64 = 7 * 1024 * 1024;

/// Fichiers des dossiers `secrets` et `cache` synchronisés (les autres restent propres au poste).
const SECRETS: [&str; 2] = ["chat_cache.key", "hyperplanning_cache.key"];
const CACHES: [&str; 3] = ["chat.wtc", "people.wtc", "hyperplanning.wtc"];

/// État d'un fichier à la dernière synchronisation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    /// Version du serveur sur laquelle repose la copie d'ici.
    version: u64,
    /// SHA-256 du contenu (en clair) à ce moment-là.
    hash: String,
    size: u64,
    /// Date de modification (ms) : fichier non relu tant qu'elle et la taille ne changent pas.
    mtime: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    /// Curseur des réceptions.
    cursor: u64,
    files: BTreeMap<String, Entry>,
}

/// Changement reçu d'un autre poste, pas encore appliqué.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Incoming {
    version: u64,
    deleted: bool,
    device: String,
}

fn manifest_path() -> PathBuf {
    dir().join("manifest.json")
}

fn incoming_path() -> PathBuf {
    dir().join("incoming.json")
}

fn load<T: Default + for<'de> Deserialize<'de>>(p: &Path) -> T {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Nom de fichier interne d'un chemin (copies de base, changements reçus).
fn slot(rel: &str) -> String {
    sha256_hex(rel.as_bytes())
}

fn base_path(rel: &str) -> PathBuf {
    dir().join("base").join(slot(rel))
}

fn staged_path(rel: &str) -> PathBuf {
    dir().join("incoming").join(slot(rel))
}

/// Écriture de l'état interne : comme `paths::write_atomic`, sans relancer la synchronisation.
fn write_quiet(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn save_quiet<T: Serialize + ?Sized>(path: &Path, v: &T) {
    if let Ok(s) = serde_json::to_vec_pretty(v) {
        let _ = write_quiet(path, &s);
    }
}

fn is_json(rel: &str) -> bool {
    rel.ends_with(".json")
}

// ───────────────────────────── Fichiers synchronisés ─────────────────────────────

/// Dossier réel d'un premier élément de chemin (`config/…`, `data/…`, `secrets/…`, `cache/…`).
fn root_of(top: &str) -> Option<PathBuf> {
    match top {
        "config" => Some(crate::paths::config()),
        "data" => Some(crate::paths::data()),
        "secrets" => Some(crate::paths::secrets()),
        "cache" => Some(crate::paths::cache()),
        _ => None,
    }
}

/// Le chemin relatif fait-il partie de ce qu'on synchronise ?
fn wanted(rel: &str) -> bool {
    let mut parts = rel.split('/');
    let top = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() || rest.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\') || p.contains(':')) {
        return false;
    }
    let name = rest.last().copied().unwrap_or("");
    if name.ends_with(".tmp") {
        return false;
    }
    match top {
        "config" => name != "crash.log",
        // Journal des automatisations : propre au poste qui les a lancées (réécrit souvent).
        "data" => rel != "data/blueprint_runs.json",
        "secrets" => rest.len() == 1 && SECRETS.contains(&name),
        "cache" => rest.len() == 1 && CACHES.contains(&name),
        _ => false,
    }
}

fn abs(rel: &str) -> Option<PathBuf> {
    if !wanted(rel) {
        return None;
    }
    let (top, rest) = rel.split_once('/')?;
    Some(rest.split('/').fold(root_of(top)?, |p, s| p.join(s)))
}

fn mtime_ms(md: &std::fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Fichiers synchronisables présents ici : chemin relatif → (chemin réel, taille, date).
fn local_files() -> BTreeMap<String, (PathBuf, u64, i64)> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, (PathBuf, u64, i64)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = format!("{rel}/{name}");
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                walk(&e.path(), &r, out);
            } else if md.is_file() && wanted(&r) {
                out.insert(r, (e.path(), md.len(), mtime_ms(&md)));
            }
        }
    }
    let mut out = BTreeMap::new();
    for top in ["config", "data", "secrets", "cache"] {
        if let Some(d) = root_of(top) {
            walk(&d, top, &mut out);
        }
    }
    out
}

/// Écrit (ou supprime) un fichier synchronisé et renvoie son entrée de manifeste.
fn write_file(rel: &str, content: Option<&[u8]>, version: u64) -> Option<Entry> {
    let path = abs(rel)?;
    match content {
        Some(c) => {
            crate::paths::write_atomic(&path, c).ok()?;
            let md = std::fs::metadata(&path).ok()?;
            Some(Entry { version, hash: sha256_hex(c), size: md.len(), mtime: mtime_ms(&md) })
        }
        None => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

fn save_base(rel: &str, content: Option<&[u8]>) {
    if !is_json(rel) {
        return;
    }
    match content {
        Some(c) => {
            let _ = write_quiet(&base_path(rel), c);
        }
        None => {
            let _ = std::fs::remove_file(base_path(rel));
        }
    }
}

/// Copie de conflit : `notes/abc.md` → `notes/abc-conflit-PC-ECOLE.md`.
fn conflict_rel(rel: &str, device: &str) -> String {
    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    let (stem, ext) = name.split_once('.').map(|(s, e)| (s, format!(".{e}"))).unwrap_or((name, String::new()));
    let device: String = device.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let mut n = 0;
    loop {
        let suffix = if n == 0 { String::new() } else { format!("-{n}") };
        let candidate = format!("{dir}/{stem}-conflit-{device}{suffix}{ext}");
        if abs(&candidate).is_some_and(|p| !p.exists()) {
            return candidate;
        }
        n += 1;
    }
}

// ───────────────────────────── Cycle ─────────────────────────────

/// Un échange complet. `startup` : l'app n'a encore rien chargé, on peut écrire les fichiers.
pub fn cycle(account: &Account, secret: &Secret, startup: bool) {
    let Some(master) = nas_sync_client::crypto::from_hex(&secret.master).and_then(|v| <[u8; 32]>::try_from(v).ok()).map(MasterKey) else {
        with_status(|s| s.phase = Phase::Error(t!("clé du compte illisible : reconnectez-vous", "account key unreadable: sign in again").into()));
        return;
    };
    let key = master.app(nas_proto::sync::WORKTABLE);
    let mut client = Client::new(&account.server, nas_proto::sync::WORKTABLE, &super::device_name()).with_token(&secret.token);
    if startup {
        client = client.connect_timeout(std::time::Duration::from_secs(3));
    }
    with_status(|s| s.phase = Phase::Syncing);
    let mut m: Manifest = load(&manifest_path());
    let mut incoming: BTreeMap<String, Incoming> = load(&incoming_path());
    let mut conflicts = Vec::new();

    // Ce qui est arrivé pendant la dernière session est appliqué avant tout.
    if startup {
        apply(&mut m, &mut incoming, &mut conflicts);
    }
    let result = (|| -> Result<(), Error> {
        receive(&client, &key, &mut m, &mut incoming)?;
        if startup {
            apply(&mut m, &mut incoming, &mut conflicts);
        }
        let skipped = send(&client, &key, &mut m, &mut incoming)?;
        if startup && !incoming.is_empty() {
            // Conflits apparus pendant l'envoi : réglés tout de suite, puis renvoyés.
            apply(&mut m, &mut incoming, &mut conflicts);
            send(&client, &key, &mut m, &mut incoming)?;
        }
        let st = client.status().ok();
        let devices = client.devices().unwrap_or_default();
        with_status(|s| {
            if let Some(st) = st {
                s.used = st.used;
                s.quota = st.quota;
                s.files = st.files;
            }
            s.devices = devices;
            s.skipped = skipped;
        });
        Ok(())
    })();
    save_quiet(&manifest_path(), &m);
    save_quiet(&incoming_path(), &incoming);
    let pending: Vec<(String, String)> = incoming.iter().map(|(rel, i)| (rel.clone(), i.device.clone())).collect();
    with_status(|s| {
        s.pending = pending;
        if startup {
            s.conflicts = conflicts;
        }
        s.connecting = false;
        s.phase = match &result {
            Ok(()) => {
                s.last_sync = Some(chrono::Local::now().timestamp_millis());
                Phase::Idle
            }
            Err(Error::Offline(e)) => Phase::Offline(e.clone()),
            Err(Error::SignedOut) => Phase::SignedOut,
            Err(e) => Phase::Error(e.to_string()),
        };
    });
}

/// Reçoit les changements des autres postes et les met de côté.
fn receive(client: &Client, key: &AppKey, m: &mut Manifest, incoming: &mut BTreeMap<String, Incoming>) -> Result<(), Error> {
    // Identifiant serveur → chemin, pour les fichiers déjà connus.
    let known: BTreeMap<String, String> = m.files.keys().chain(incoming.keys()).map(|rel| (key.doc_id(rel), rel.clone())).collect();
    loop {
        let pull = client.pull(m.cursor)?;
        for meta in &pull.docs {
            stage(client, key, m, incoming, &known, meta)?;
        }
        m.cursor = pull.cursor;
        if !pull.more {
            return Ok(());
        }
    }
}

fn stage(client: &Client, key: &AppKey, m: &Manifest, incoming: &mut BTreeMap<String, Incoming>, known: &BTreeMap<String, String>, meta: &DocMeta) -> Result<(), Error> {
    // Notre propre envoi, ou version déjà en attente.
    if let Some(rel) = known.get(&meta.doc)
        && (m.files.get(rel).is_some_and(|e| e.version >= meta.version) || incoming.get(rel).is_some_and(|i| i.version >= meta.version))
    {
        return Ok(());
    }
    let (rel, content) = if meta.deleted {
        match known.get(&meta.doc) {
            Some(rel) => (rel.clone(), None),
            // Supprimé avant même d'arriver ici : rien à faire.
            None => return Ok(()),
        }
    } else {
        let (rel, c) = client.fetch(key, meta)?;
        (rel, Some(c))
    };
    if !wanted(&rel) {
        return Ok(());
    }
    match &content {
        Some(c) => {
            if write_quiet(&staged_path(&rel), c).is_err() {
                return Err(Error::Other(t!("dossier de synchronisation inaccessible", "sync folder unavailable").into()));
            }
        }
        None => {
            let _ = std::fs::remove_file(staged_path(&rel));
        }
    }
    incoming.insert(rel, Incoming { version: meta.version, deleted: meta.deleted, device: meta.device.clone() });
    Ok(())
}

/// Applique les changements mis de côté (au démarrage seulement), en fusionnant avec ceux d'ici.
fn apply(m: &mut Manifest, incoming: &mut BTreeMap<String, Incoming>, conflicts: &mut Vec<String>) {
    for (rel, inc) in std::mem::take(incoming) {
        let remote = if inc.deleted { None } else { std::fs::read(staged_path(&rel)).ok() };
        if !inc.deleted && remote.is_none() {
            continue; // contenu perdu : il reviendra au prochain échange complet
        }
        let local = abs(&rel).and_then(|p| std::fs::read(p).ok());
        let entry = m.files.get(&rel).cloned();
        let unchanged_here = match (&local, &entry) {
            (Some(c), Some(e)) => sha256_hex(c) == e.hash,
            (None, None) => true,
            _ => false,
        };
        if unchanged_here || local == remote {
            set(m, &rel, write_file(&rel, remote.as_deref(), inc.version));
            save_base(&rel, remote.as_deref());
        } else {
            match (&local, &remote) {
                // Modifié des deux côtés, JSON : fusion.
                (Some(l), Some(r)) if is_json(&rel) => {
                    let parse = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).ok();
                    let base = std::fs::read(base_path(&rel)).ok().and_then(|b| parse(&b));
                    match (parse(l), parse(r)) {
                        (Some(lv), Some(rv)) => {
                            let merged = serde_json::to_vec_pretty(&merge::merge(base.as_ref(), &lv, &rv)).unwrap_or_else(|_| l.clone());
                            // Repose sur la version reçue ; la fusion sera envoyée comme un changement d'ici.
                            if write_file(&rel, Some(&merged), inc.version).is_some() {
                                m.files.insert(rel.clone(), Entry { version: inc.version, hash: sha256_hex(r), size: 0, mtime: 0 });
                            }
                            save_base(&rel, Some(r));
                        }
                        _ => keep_both(m, &rel, l, r, &inc, conflicts),
                    }
                }
                // Modifié des deux côtés, autre type : on garde les deux.
                (Some(l), Some(r)) => keep_both(m, &rel, l, r, &inc, conflicts),
                // Supprimé là-bas mais modifié ici : on le garde (renvoyé comme modification).
                (Some(_), None) => {
                    m.files.insert(rel.clone(), Entry { version: inc.version, hash: String::new(), size: 0, mtime: 0 });
                }
                // Supprimé ici mais modifié là-bas : la modification l'emporte.
                (None, Some(r)) => {
                    set(m, &rel, write_file(&rel, Some(r), inc.version));
                    save_base(&rel, Some(r));
                }
                (None, None) => {
                    m.files.remove(&rel);
                }
            }
        }
        let _ = std::fs::remove_file(staged_path(&rel));
    }
}

fn set(m: &mut Manifest, rel: &str, e: Option<Entry>) {
    match e {
        Some(e) => {
            m.files.insert(rel.to_string(), e);
        }
        None => {
            m.files.remove(rel);
        }
    }
}

/// Garde la version reçue sous le nom d'origine et celle d'ici dans une copie « conflit ».
fn keep_both(m: &mut Manifest, rel: &str, local: &[u8], remote: &[u8], inc: &Incoming, conflicts: &mut Vec<String>) {
    let copy = conflict_rel(rel, &super::device_name());
    if let Some(p) = abs(&copy)
        && crate::paths::write_atomic(&p, local).is_ok()
    {
        conflicts.push(copy);
        set(m, rel, write_file(rel, Some(remote), inc.version));
    }
}

/// Envoie ce qui a changé ici. Renvoie les fichiers trop gros.
fn send(client: &Client, key: &AppKey, m: &mut Manifest, incoming: &mut BTreeMap<String, Incoming>) -> Result<Vec<String>, Error> {
    let local = local_files();
    let mut skipped = Vec::new();
    for (rel, (path, size, mtime)) in &local {
        if incoming.contains_key(rel) {
            continue; // attend d'être fusionné au démarrage
        }
        let entry = m.files.get(rel).cloned();
        if entry.as_ref().is_some_and(|e| e.size == *size && e.mtime == *mtime) {
            continue;
        }
        if *size > MAX_FILE {
            skipped.push(rel.clone());
            continue;
        }
        let Ok(content) = std::fs::read(path) else { continue };
        let hash = sha256_hex(&content);
        if let Some(e) = &entry
            && e.hash == hash
        {
            m.files.insert(rel.clone(), Entry { size: *size, mtime: *mtime, ..e.clone() });
            continue;
        }
        let base = entry.as_ref().map(|e| e.version).unwrap_or(0);
        let r = client.push(key, rel, base, Some(&content))?;
        if r.ok {
            m.files.insert(rel.clone(), Entry { version: r.version, hash, size: *size, mtime: *mtime });
            save_base(rel, Some(&content));
        } else if let Some(meta) = r.conflict {
            // Modifié ailleurs entre-temps : version reçue mise de côté, fusion au démarrage.
            let known = BTreeMap::from([(meta.doc.clone(), rel.clone())]);
            stage(client, key, m, incoming, &known, &meta)?;
        } else if !r.message.is_empty() {
            return Err(Error::Other(r.message));
        }
        save_quiet(&manifest_path(), &*m);
    }
    // Fichiers supprimés ici.
    let gone: Vec<(String, Entry)> = m.files.iter().filter(|(rel, _)| !local.contains_key(*rel) && !incoming.contains_key(*rel)).map(|(r, e)| (r.clone(), e.clone())).collect();
    for (rel, e) in gone {
        let r = client.push(key, &rel, e.version, None)?;
        if r.ok {
            m.files.remove(&rel);
            save_base(&rel, None);
        } else if let Some(meta) = r.conflict {
            let known = BTreeMap::from([(meta.doc.clone(), rel.clone())]);
            stage(client, key, m, incoming, &known, &meta)?;
        }
    }
    Ok(skipped)
}

/// Sauvegarde zip des données de ce poste avant la première synchronisation (filet de sécurité).
pub fn backup_before_first_sync() {
    let target = dir().join(format!("avant-synchro-{}.zip", chrono::Local::now().format("%Y-%m-%d-%H%M")));
    let _ = std::fs::create_dir_all(dir());
    let Ok(file) = std::fs::File::create(&target) else { return };
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (rel, (path, ..)) in local_files() {
        if let Ok(bytes) = std::fs::read(&path)
            && zip.start_file(rel.as_str(), options).is_ok()
        {
            use std::io::Write;
            let _ = zip.write_all(&bytes);
        }
    }
    let _ = zip.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_paths() {
        assert!(wanted("data/notes/abc.md"));
        assert!(wanted("config/tasks.json"));
        assert!(wanted("cache/chat.wtc"));
        assert!(wanted("secrets/chat_cache.key"));
        assert!(!wanted("secrets/google_refresh_token"));
        assert!(!wanted("cache/emoji/1f600.png"));
        assert!(!wanted("config/crash.log"));
        assert!(!wanted("config/tasks.json.tmp"));
        assert!(!wanted("data/../secrets/x"));
        assert!(!wanted("sync/manifest.json"));
        assert!(!wanted("data"));
        assert!(!wanted("data/blueprint_runs.json"));
    }

    #[test]
    fn conflict_names() {
        let c = conflict_rel("data/notes/abc.locked.json", "PC ECOLE");
        assert!(c.starts_with("data/notes/abc-conflit-PC_ECOLE"));
        assert!(c.ends_with(".locked.json"));
    }
}
