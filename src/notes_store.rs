//! Notes locales : un fichier Markdown par note (compatible Obsidian), images dans `attachments/`.
//!
//! Dossier : `data\data\notes\` (dossier `data` du projet, voir `paths`)
//! - `<id>.md` : en-tête (titre, dates, favori, lecture seule, liens) puis le texte en Markdown ;
//! - `<id>.locked.json` : note protégée par mot de passe — texte, liens et images chiffrés
//!   (AES-256-GCM, clé dérivée du mot de passe par Argon2id). Seul le titre reste lisible.

use std::collections::HashMap;
use std::path::PathBuf;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Text,
    H1,
    H2,
    H3,
    Bullet,
    Numbered,
    Todo(bool),
    Quote,
    Code,
    Divider,
    /// Image : `text` = chemin relatif (attachments/…).
    Image,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => t!("Texte", "Text"),
            Kind::H1 => t!("Titre 1", "Heading 1"),
            Kind::H2 => t!("Titre 2", "Heading 2"),
            Kind::H3 => t!("Titre 3", "Heading 3"),
            Kind::Bullet => t!("Liste à puces", "Bulleted list"),
            Kind::Numbered => t!("Liste numérotée", "Numbered list"),
            Kind::Todo(_) => t!("Case à cocher", "Checkbox"),
            Kind::Quote => t!("Citation", "Quote"),
            Kind::Code => "Code",
            Kind::Divider => t!("Séparateur", "Divider"),
            Kind::Image => "Image",
        }
    }
    /// Un retour à la ligne crée un bloc du même type (listes).
    pub fn continues(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Numbered | Kind::Todo(_))
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: u64,
    pub kind: Kind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NoteLink {
    Event {
        calendar: String,
        id: String,
        title: String,
        start: i64,
    },
    Date {
        date: String,
    },
    Mail {
        id: String,
        subject: String,
        from: String,
    },
    Drive {
        id: String,
        name: String,
        mime: String,
        url: Option<String>,
    },
    /// Conversation Google Chat (et éventuellement un message précis).
    Chat {
        space: String,
        message: Option<String>,
        title: String,
        text: String,
    },
    Task {
        id: u64,
        title: String,
    },
}

impl NoteLink {
    /// Clé stable, utilisée dans l'adresse des mentions (`wt:<clé>`).
    pub fn key(&self) -> String {
        match self {
            NoteLink::Event { id, .. } => format!("event:{id}"),
            NoteLink::Date { date } => format!("date:{date}"),
            NoteLink::Mail { id, .. } => format!("mail:{id}"),
            NoteLink::Drive { id, .. } => format!("drive:{id}"),
            NoteLink::Chat { space, message, .. } => format!("chat:{}", message.as_deref().unwrap_or(space)),
            NoteLink::Task { id, .. } => format!("task:{id}"),
        }
    }
    /// Texte de la mention insérée dans la note.
    pub fn mention(&self) -> String {
        match self {
            NoteLink::Event { title, .. } => format!("🗓 {title}"),
            NoteLink::Date { date } => format!("📆 {date}"),
            NoteLink::Mail { subject, .. } => format!("✉ {subject}"),
            NoteLink::Drive { name, .. } => format!("📄 {name}"),
            NoteLink::Chat { title, text, .. } => {
                if text.is_empty() {
                    format!("💬 {title}")
                } else {
                    format!("💬 {title} : {}", text.chars().take(40).collect::<String>())
                }
            }
            NoteLink::Task { title, .. } => format!("✅ {title}"),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Header {
    title: String,
    created: i64,
    updated: i64,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    readonly: bool,
    #[serde(default)]
    links: Vec<NoteLink>,
}

/// Contenu chiffré d'une note protégée.
#[derive(Serialize, Deserialize)]
struct LockedFile {
    #[serde(flatten)]
    header: Header,
    salt: String,
    nonce: String,
    data: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Secret {
    body: String,
    links: Vec<NoteLink>,
    /// Images de la note (chemin relatif → contenu en base64).
    images: HashMap<String, String>,
}

pub struct Note {
    pub id: String,
    pub title: String,
    pub created: i64,
    pub updated: i64,
    pub favorite: bool,
    pub readonly: bool,
    pub links: Vec<NoteLink>,
    pub blocks: Vec<Block>,
    /// Protégée par mot de passe (fichier .locked.json).
    pub protected: bool,
    /// Clé de la note déverrouillée pendant la session (jamais écrite).
    key: Option<([u8; 32], Vec<u8>)>,
    /// Données chiffrées tant que la note n'est pas déverrouillée.
    sealed: Option<(String, String)>,
    /// Images d'une note protégée (en mémoire uniquement).
    pub images: HashMap<String, Vec<u8>>,
    pub dirty: bool,
    pub edited_at: Option<std::time::Instant>,
    next_block: u64,
    /// Note intégrée à l'app (le guide) : en lecture seule, jamais enregistrée ni supprimée.
    pub builtin: bool,
}

/// Recolle les paragraphes coupés à la main dans un Markdown (une ligne du fichier = un bloc
/// dans l'éditeur) : une ligne de texte qui suit une ligne de texte ou d'élément de liste la
/// continue. Les titres, listes, citations, tableaux, séparateurs et blocs de code restent tels quels.
fn unwrap_lines(md: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut code = false;
    let mut joinable = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") {
            code = !code;
            out.push(line.to_string());
            joinable = false;
            continue;
        }
        if code {
            out.push(line.to_string());
            continue;
        }
        let starts_block = t.is_empty()
            || t.starts_with('#')
            || t.starts_with('>')
            || t.starts_with('|')
            || t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("---")
            || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        match out.last_mut() {
            Some(prev) if joinable && !starts_block => {
                prev.push(' ');
                prev.push_str(t);
            }
            _ => out.push(line.to_string()),
        }
        // Texte ou élément de liste : la ligne suivante peut le continuer.
        joinable = !t.is_empty() && !t.starts_with('#') && !t.starts_with('|') && !t.starts_with("---");
    }
    out.join("\n")
}

/// Identifiant du guide intégré.
pub const GUIDE_ID: &str = "worktable-guide";

/// Le guide de l'app : le `README.md` du projet, compilé dans l'exécutable. Il suit donc le
/// dépôt : chaque version publiée affiche le README de son commit.
const README: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"));

pub fn dir() -> Option<PathBuf> {
    Some(crate::paths::data().join("notes"))
}

/// Fichier d'une image de note (`rel` : chemin écrit dans la note, relatif au dossier des notes
/// ou absolu).
pub fn image_path(rel: &str) -> Option<PathBuf> {
    let rel = rel.replace("%20", " ");
    let p = std::path::Path::new(&rel);
    if p.is_absolute() { Some(p.to_path_buf()) } else { Some(dir()?.join(p)) }
}

fn now() -> i64 {
    Local::now().timestamp_millis()
}

impl Note {
    pub fn new() -> Self {
        let n = now();
        let mut note = Note {
            id: format!("{}", Local::now().format("%Y%m%d-%H%M%S-%3f")),
            title: String::new(),
            created: n,
            updated: n,
            favorite: false,
            readonly: false,
            links: Vec::new(),
            blocks: Vec::new(),
            protected: false,
            key: None,
            sealed: None,
            images: HashMap::new(),
            dirty: true,
            edited_at: None,
            next_block: 0,
            builtin: false,
        };
        let b = note.block(Kind::Text, "");
        note.blocks.push(b);
        note
    }

    /// Le guide intégré (README), en favori et en lecture seule.
    pub fn guide() -> Self {
        let mut n = Note::new();
        n.id = GUIDE_ID.into();
        n.title = t!("Guide de WorkTable (README)", "WorkTable guide (README)").into();
        // Le titre « # WorkTable » du README fait doublon avec celui de la note.
        let body = README.replace("\r\n", "\n");
        let body = body.strip_prefix("# WorkTable\n").unwrap_or(&body).trim_start_matches('\n');
        n.set_markdown(&unwrap_lines(body));
        n.created = 0;
        n.updated = 0;
        n.favorite = true;
        n.readonly = true;
        n.builtin = true;
        n.dirty = false;
        n
    }

    pub fn block(&mut self, kind: Kind, text: &str) -> Block {
        self.next_block += 1;
        Block { id: self.next_block, kind, text: text.to_string() }
    }

    /// Verrouillée = protégée et pas encore déverrouillée pendant cette session.
    pub fn locked(&self) -> bool {
        self.protected && self.key.is_none()
    }

    pub fn touch(&mut self) {
        self.updated = now();
        self.dirty = true;
        self.edited_at = Some(std::time::Instant::now());
    }

    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() { t!("Sans titre", "Untitled").into() } else { self.title.clone() }
    }

    /// Texte brut (aperçu, recherche) — vide si verrouillée.
    pub fn plain_text(&self) -> String {
        if self.locked() {
            return String::new();
        }
        self.blocks.iter().filter(|b| b.kind != Kind::Image && b.kind != Kind::Divider).map(|b| b.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    // --- Markdown ----------------------------------------------------------------------

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let mut n = 0;
        for b in &self.blocks {
            if b.kind == Kind::Numbered {
                n += 1;
            } else {
                n = 0;
            }
            let line = match b.kind {
                Kind::Text => b.text.clone(),
                Kind::H1 => format!("# {}", b.text),
                Kind::H2 => format!("## {}", b.text),
                Kind::H3 => format!("### {}", b.text),
                Kind::Bullet => format!("- {}", b.text),
                Kind::Numbered => format!("{n}. {}", b.text),
                Kind::Todo(done) => format!("- [{}] {}", if done { "x" } else { " " }, b.text),
                Kind::Quote => format!("> {}", b.text),
                Kind::Code => format!("```\n{}\n```", b.text),
                Kind::Divider => "---".into(),
                Kind::Image => format!("![]({})", b.text.replace(' ', "%20")),
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    pub fn set_markdown(&mut self, md: &str) {
        self.blocks.clear();
        let mut lines = md.lines().peekable();
        while let Some(line) = lines.next() {
            if line.trim_start().starts_with("```") {
                let mut code = Vec::new();
                for l in lines.by_ref() {
                    if l.trim_start().starts_with("```") {
                        break;
                    }
                    code.push(l);
                }
                let b = self.block(Kind::Code, &code.join("\n"));
                self.blocks.push(b);
                continue;
            }
            let (kind, text) = parse_line(line);
            let b = self.block(kind, &text);
            self.blocks.push(b);
        }
        if self.blocks.is_empty() {
            let b = self.block(Kind::Text, "");
            self.blocks.push(b);
        }
    }

    // --- Fichiers -----------------------------------------------------------------------

    fn header(&self) -> Header {
        Header {
            title: self.title.clone(),
            created: self.created,
            updated: self.updated,
            favorite: self.favorite,
            readonly: self.readonly,
            links: if self.protected { Vec::new() } else { self.links.clone() },
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        if self.builtin {
            self.dirty = false;
            return Ok(());
        }
        let d = dir().ok_or(t!("dossier introuvable", "folder not found"))?;
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let md_path = d.join(format!("{}.md", self.id));
        let locked_path = d.join(format!("{}.locked.json", self.id));
        if self.protected {
            let header = self.header();
            let (data, nonce, salt) = match (&self.key, &self.sealed) {
                (Some((key, salt)), _) => {
                    let secret = Secret { body: self.to_markdown(), links: self.links.clone(), images: self.images.iter().map(|(k, v)| (k.clone(), B64.encode(v))).collect() };
                    let plain = serde_json::to_vec(&secret).map_err(|e| e.to_string())?;
                    let (nonce, ct) = encrypt(key, &plain)?;
                    (B64.encode(ct), B64.encode(nonce), B64.encode(salt))
                }
                // Verrouillée : on ne réécrit que l'en-tête (favori…) autour des données chiffrées.
                (None, Some((data, nonce))) => {
                    let old: LockedFile = serde_json::from_str(&std::fs::read_to_string(&locked_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    (data.clone(), nonce.clone(), old.salt)
                }
                _ => return Err(t!("note verrouillée", "note is locked").into()),
            };
            let file = LockedFile { header, salt, nonce: nonce.clone(), data: data.clone() };
            write_atomic(&locked_path, serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?.as_bytes())?;
            if self.key.is_some() {
                self.sealed = Some((data, nonce));
            }
            let _ = std::fs::remove_file(md_path);
        } else {
            let h = serde_json::to_string(&self.header()).map_err(|e| e.to_string())?;
            let text = format!("---\nworktable: {h}\n---\n{}", self.to_markdown());
            write_atomic(&md_path, text.as_bytes())?;
            let _ = std::fs::remove_file(locked_path);
        }
        self.dirty = false;
        Ok(())
    }

    pub fn delete(&self) {
        if let Some(d) = dir() {
            let _ = std::fs::remove_file(d.join(format!("{}.md", self.id)));
            let _ = std::fs::remove_file(d.join(format!("{}.locked.json", self.id)));
            let _ = std::fs::remove_dir_all(d.join("attachments").join(&self.id));
        }
    }

    fn load_md(id: &str, text: &str) -> Note {
        let mut note = Note::new();
        note.id = id.to_string();
        note.dirty = false;
        // Fins de ligne Windows (git autocrlf, éditeurs) et BOM : ramenées au format d'origine.
        let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let text = text.as_str();
        let mut body = text;
        if let Some(rest) = text.strip_prefix("---\n")
            && let Some(end) = rest.find("\n---\n")
        {
            let head = &rest[..end];
            body = &rest[end + 5..];
            if let Some(json) = head.lines().find_map(|l| l.strip_prefix("worktable: "))
                && let Ok(h) = serde_json::from_str::<Header>(json)
            {
                note.title = h.title;
                note.created = h.created;
                note.updated = h.updated;
                note.favorite = h.favorite;
                note.readonly = h.readonly;
                note.links = h.links;
            }
        }
        if note.title.is_empty() {
            note.title = id.to_string();
        }
        note.set_markdown(body);
        note
    }

    fn load_locked(id: &str, text: &str) -> Option<Note> {
        let f: LockedFile = serde_json::from_str(text).ok()?;
        let mut note = Note::new();
        note.id = id.to_string();
        note.dirty = false;
        note.title = f.header.title;
        note.created = f.header.created;
        note.updated = f.header.updated;
        note.favorite = f.header.favorite;
        note.readonly = f.header.readonly;
        note.protected = true;
        note.blocks.clear();
        note.sealed = Some((f.data, f.nonce));
        Some(note)
    }

    /// Toutes les notes du dossier (plus récentes d'abord).
    pub fn load_all() -> Vec<Note> {
        let Some(d) = dir() else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(&d) else { return Vec::new() };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            if let Some(id) = name.strip_suffix(".locked.json") {
                if let Some(n) = Note::load_locked(id, &text) {
                    out.push(n);
                }
            } else if let Some(id) = name.strip_suffix(".md") {
                out.push(Note::load_md(id, &text));
            }
        }
        out.sort_by_key(|n| -n.updated);
        out.push(Note::guide());
        out
    }

    // --- Mot de passe -------------------------------------------------------------------

    /// Protège la note : texte, liens et images chiffrés ; les images quittent le disque en clair.
    pub fn protect(&mut self, password: &str) -> Result<(), String> {
        if password.chars().count() < 4 {
            return Err(t!("Mot de passe trop court (4 caractères minimum)", "Password too short (4 characters minimum)").into());
        }
        let mut salt = vec![0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let key = derive(password, &salt)?;
        // Images et fichiers joints : chargés en mémoire (chiffrés avec la note) puis retirés du dossier.
        let att = dir().map(|d| d.join("attachments").join(&self.id));
        if let Some(rd) = att.as_ref().and_then(|a| std::fs::read_dir(a).ok()) {
            for e in rd.flatten() {
                if let Ok(bytes) = std::fs::read(e.path()) {
                    self.images.insert(format!("attachments/{}/{}", self.id, e.file_name().to_string_lossy()), bytes);
                }
            }
        }
        self.key = Some((key, salt));
        self.protected = true;
        self.touch();
        self.save()?;
        if let Some(a) = att {
            let _ = std::fs::remove_dir_all(a);
        }
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<(), String> {
        let d = dir().ok_or(t!("dossier introuvable", "folder not found"))?;
        let f: LockedFile = serde_json::from_str(&std::fs::read_to_string(d.join(format!("{}.locked.json", self.id))).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let salt = B64.decode(&f.salt).map_err(|e| e.to_string())?;
        let key = derive(password, &salt)?;
        let nonce = B64.decode(&f.nonce).map_err(|e| e.to_string())?;
        let data = B64.decode(&f.data).map_err(|e| e.to_string())?;
        let plain = decrypt(&key, &nonce, &data).map_err(|_| t!("Mot de passe incorrect", "Wrong password").to_string())?;
        let secret: Secret = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;
        self.set_markdown(&secret.body);
        self.links = secret.links;
        self.images = secret.images.into_iter().filter_map(|(k, v)| B64.decode(v).ok().map(|b| (k, b))).collect();
        self.key = Some((key, salt));
        Ok(())
    }

    /// Reverrouille (oublie la clé et le contenu en mémoire).
    pub fn relock(&mut self) {
        if !self.protected {
            return;
        }
        if self.dirty {
            let _ = self.save();
        }
        self.key = None;
        self.blocks.clear();
        self.links.clear();
        self.images.clear();
    }

    /// Retire la protection (la note doit être déverrouillée) : images réécrites sur le disque.
    pub fn unprotect(&mut self) -> Result<(), String> {
        if self.locked() {
            return Err(t!("Déverrouille d'abord la note", "Unlock the note first").into());
        }
        if let Some(a) = dir().map(|d| d.join("attachments").join(&self.id)) {
            let _ = std::fs::create_dir_all(&a);
            for (path, bytes) in &self.images {
                let file = path.rsplit('/').next().unwrap_or(path);
                let _ = std::fs::write(a.join(file), bytes);
            }
        }
        self.images.clear();
        self.protected = false;
        self.key = None;
        self.sealed = None;
        self.touch();
        self.save()
    }

    /// Ajoute une image (copiée dans attachments/<id>/) et renvoie son chemin relatif.
    pub fn add_image(&mut self, src: &std::path::Path) -> Result<String, String> {
        let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image.png".into());
        self.add_file_bytes(&name, bytes)
    }

    pub fn add_file_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<String, String> {
        let safe: String = name.chars().map(|c| if r#"<>:"/\|?* "#.contains(c) || c.is_control() { '_' } else { c }).collect();
        let file = format!("{}-{safe}", Local::now().format("%H%M%S%3f"));
        let rel = format!("attachments/{}/{file}", self.id);
        if self.protected {
            self.images.insert(rel.clone(), bytes);
        } else {
            let d = dir().ok_or(t!("dossier introuvable", "folder not found"))?.join("attachments").join(&self.id);
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            std::fs::write(d.join(&file), bytes).map_err(|e| e.to_string())?;
        }
        Ok(rel)
    }

    /// Début des données chiffrées (affichées floutées tant que la note est verrouillée).
    pub fn cipher_preview(&self, n: usize) -> String {
        self.sealed.as_ref().map(|(d, _)| d.chars().take(n).collect()).unwrap_or_default()
    }
}

pub fn parse_line(line: &str) -> (Kind, String) {
    let t = line;
    if let Some(r) = t.strip_prefix("### ") {
        return (Kind::H3, r.into());
    }
    if let Some(r) = t.strip_prefix("## ") {
        return (Kind::H2, r.into());
    }
    if let Some(r) = t.strip_prefix("# ") {
        return (Kind::H1, r.into());
    }
    for (p, done) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
        if let Some(r) = t.strip_prefix(p) {
            return (Kind::Todo(done), r.into());
        }
    }
    if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return (Kind::Bullet, r.into());
    }
    if let Some((n, r)) = t.split_once(". ")
        && !n.is_empty()
        && n.len() <= 3
        && n.chars().all(|c| c.is_ascii_digit())
    {
        return (Kind::Numbered, r.into());
    }
    if let Some(r) = t.strip_prefix("> ") {
        return (Kind::Quote, r.into());
    }
    if t.trim() == "---" || t.trim() == "***" {
        return (Kind::Divider, String::new());
    }
    if let Some(r) = t.strip_prefix("![")
        && let Some((_, rest)) = r.split_once("](")
        && let Some(path) = rest.strip_suffix(')')
    {
        return (Kind::Image, path.replace("%20", " "));
    }
    (Kind::Text, t.into())
}

fn derive(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    argon2::Argon2::default().hash_password_into(password.as_bytes(), salt, &mut key).map_err(|e| e.to_string())?;
    Ok(key)
}

pub(crate) fn encrypt(key: &[u8; 32], plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain).map_err(|e| e.to_string())?;
    Ok((nonce.to_vec(), ct))
}

pub(crate) fn decrypt(key: &[u8; 32], nonce: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if nonce.len() != 12 {
        return Err(t!("données invalides", "invalid data").into());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    cipher.decrypt(Nonce::from_slice(nonce), data).map_err(|e| e.to_string())
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    crate::paths::write_atomic(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_round_trip() {
        let mut n = Note::new();
        n.set_markdown("# Titre\nTexte **gras**\n- puce\n1. un\n2. deux\n- [x] fait\n> cite\n---\n```\nlet a = 1;\nlet b = 2;\n```\n![](attachments/x/img.png)\n");
        let kinds: Vec<Kind> = n.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [Kind::H1, Kind::Text, Kind::Bullet, Kind::Numbered, Kind::Numbered, Kind::Todo(true), Kind::Quote, Kind::Divider, Kind::Code, Kind::Image]);
        assert_eq!(n.blocks[8].text, "let a = 1;\nlet b = 2;");
        let md = n.to_markdown();
        let mut m = Note::new();
        m.set_markdown(&md);
        assert_eq!(m.to_markdown(), md);
    }

    #[test]
    fn readme_paragraphs_are_rejoined() {
        let md = "Un texte\ncoupé en deux.\n\n- liste\n  suite\n- autre\n```\na\nb\n```\n## Titre\nfin";
        assert_eq!(unwrap_lines(md), "Un texte coupé en deux.\n\n- liste suite\n- autre\n```\na\nb\n```\n## Titre\nfin");
    }

    #[test]
    fn guide_is_the_readme_read_only() {
        let mut g = Note::guide();
        assert!(g.builtin && g.readonly && g.favorite);
        assert!(g.blocks.iter().any(|b| b.kind == Kind::H2 && b.text.starts_with("Blueprint")));
        // Jamais écrit sur le disque.
        g.dirty = true;
        assert!(g.save().is_ok());
        assert!(!g.dirty);
    }

    #[test]
    fn encryption_round_trip() {
        let key = derive("secret", b"0123456789abcdef").unwrap();
        let (nonce, ct) = encrypt(&key, b"bonjour").unwrap();
        assert_eq!(decrypt(&key, &nonce, &ct).unwrap(), b"bonjour");
        let wrong = derive("autre", b"0123456789abcdef").unwrap();
        assert!(decrypt(&wrong, &nonce, &ct).is_err());
        assert!(!String::from_utf8_lossy(&ct).contains("bonjour"));
    }

    #[test]
    fn crlf_front_matter() {
        let text =
            "\u{feff}---\r\nworktable: {\"title\":\"Transfert\",\"created\":1,\"updated\":2,\"favorite\":true,\"readonly\":false,\"links\":[]}\r\n---\r\n# Titre\r\nligne\r\n";
        let n = Note::load_md("x", text);
        assert_eq!(n.title, "Transfert");
        assert!(n.favorite);
        assert!(!n.to_markdown().contains('\r'));
        assert!(!n.to_markdown().contains("worktable"));
    }
}
