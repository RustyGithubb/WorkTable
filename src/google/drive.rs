//! Appels à l'API Google Drive v3 (lecture seule) : dossiers, recherche, téléchargement / export.

use chrono::DateTime;
use serde_json::Value;

use super::Session;

const API: &str = "https://www.googleapis.com/drive/v3";
const FIELDS: &str = "nextPageToken,files(id,name,mimeType,modifiedTime,size,owners(displayName,emailAddress),\
lastModifyingUser(displayName),thumbnailLink,webViewLink,starred,shared,parents,driveId,hasThumbnail)";
/// Taille maximale d'un fichier téléchargé pour l'ouvrir sur le PC.
const MAX_DOWNLOAD: u64 = 500 * 1024 * 1024;
pub const FOLDER: &str = "application/vnd.google-apps.folder";

#[derive(Clone, Debug, Default)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime: String,
    /// Dernière modification (ms).
    pub modified: i64,
    pub size: Option<u64>,
    pub owner: String,
    pub modified_by: String,
    pub thumbnail: Option<String>,
    pub web_link: Option<String>,
    pub starred: bool,
    pub shared: bool,
    pub parents: Vec<String>,
}

impl DriveFile {
    pub fn is_folder(&self) -> bool {
        self.mime == FOLDER
    }
    /// Document Google (Docs, Sheets…) : pas de contenu binaire, export ou navigateur.
    pub fn is_google(&self) -> bool {
        self.mime.starts_with("application/vnd.google-apps.")
    }
    pub fn extension(&self) -> String {
        self.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct SharedDrive {
    pub id: String,
    pub name: String,
}

/// Contenu à lister.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    MyDrive,
    Folder(String),
    Shared,
    Recent,
    Starred,
    SharedDrive(String),
    Search(String),
}

pub enum DriveEvent {
    Listing {
        source: Source,
        files: Vec<DriveFile>,
        next: Option<String>,
        append: bool,
    },
    Drives(Vec<SharedDrive>),
    /// Chemin d'un dossier (de la racine au dossier).
    Path {
        folder: String,
        path: Vec<(String, String)>,
    },
    Thumb {
        key: String,
        image: Result<std::sync::Arc<eframe::egui::ColorImage>, String>,
    },
    /// Fichier prêt sur le disque : ouvert avec l'application par défaut.
    Downloaded {
        path: std::path::PathBuf,
        open: bool,
    },
    /// Recherche de la barre globale.
    Found {
        query: String,
        files: Vec<DriveFile>,
    },
    Error(String),
}

fn ms(t: &str) -> i64 {
    DateTime::parse_from_rfc3339(t).map(|d| d.timestamp_millis()).unwrap_or(0)
}

fn parse(v: &Value) -> DriveFile {
    DriveFile {
        id: v["id"].as_str().unwrap_or_default().into(),
        name: v["name"].as_str().unwrap_or_default().into(),
        mime: v["mimeType"].as_str().unwrap_or_default().into(),
        modified: v["modifiedTime"].as_str().map(ms).unwrap_or(0),
        size: v["size"].as_str().and_then(|s| s.parse().ok()),
        owner: v["owners"][0]["displayName"].as_str().unwrap_or_default().into(),
        modified_by: v["lastModifyingUser"]["displayName"].as_str().unwrap_or_default().into(),
        thumbnail: v["thumbnailLink"].as_str().filter(|_| v["hasThumbnail"].as_bool().unwrap_or(true)).map(str::to_string),
        web_link: v["webViewLink"].as_str().map(str::to_string),
        starred: v["starred"].as_bool().unwrap_or(false),
        shared: v["shared"].as_bool().unwrap_or(false),
        parents: v["parents"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Une page de fichiers pour une source (100 éléments).
pub fn list(s: &Session, source: &Source, page: Option<&str>) -> Result<(Vec<DriveFile>, Option<String>), String> {
    let (q, order, corpora, drive): (String, &str, &str, Option<&str>) = match source {
        Source::MyDrive => ("'root' in parents and trashed = false".into(), "folder,name_natural", "user", None),
        Source::Folder(id) => (format!("'{}' in parents and trashed = false", escape(id)), "folder,name_natural", "allDrives", None),
        Source::Shared => ("sharedWithMe and trashed = false".into(), "sharedWithMeTime desc", "user", None),
        Source::Recent => (format!("trashed = false and mimeType != '{FOLDER}'"), "viewedByMeTime desc", "user", None),
        Source::Starred => ("starred and trashed = false".into(), "folder,name_natural", "allDrives", None),
        Source::SharedDrive(id) => (format!("'{}' in parents and trashed = false", escape(id)), "folder,name_natural", "drive", Some(id.as_str())),
        Source::Search(text) => {
            let t = escape(text.trim());
            (format!("trashed = false and (name contains '{t}' or fullText contains '{t}')"), "", "allDrives", None)
        }
    };
    let mut query: Vec<(&str, &str)> =
        vec![("q", q.as_str()), ("fields", FIELDS), ("pageSize", "100"), ("supportsAllDrives", "true"), ("includeItemsFromAllDrives", "true"), ("corpora", corpora)];
    if !order.is_empty() {
        query.push(("orderBy", order));
    }
    if let Some(d) = drive {
        query.push(("driveId", d));
    }
    if let Some(p) = page {
        query.push(("pageToken", p));
    }
    let v = s.get(&format!("{API}/files"), &query)?;
    let files = v["files"].as_array().into_iter().flatten().map(parse).collect();
    let next = v["nextPageToken"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
    Ok((files, next))
}

pub fn shared_drives(s: &Session) -> Result<Vec<SharedDrive>, String> {
    let v = s.get(&format!("{API}/drives"), &[("pageSize", "100")])?;
    Ok(v["drives"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| SharedDrive { id: d["id"].as_str().unwrap_or_default().into(), name: d["name"].as_str().unwrap_or_default().into() })
        .collect())
}

/// Chemin d'un dossier : remonte les parents (au plus 12 niveaux).
pub fn path_of(s: &Session, folder: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut cur = folder.to_string();
    for _ in 0..12 {
        let v = s.get(&format!("{API}/files/{cur}"), &[("fields", "id,name,parents,driveId"), ("supportsAllDrives", "true")])?;
        let name = v["name"].as_str().unwrap_or_default().to_string();
        let parent = v["parents"][0].as_str().map(str::to_string);
        if parent.is_none() {
            // Racine : « Mon Drive » ou un Drive partagé.
            break;
        }
        out.push((cur.clone(), name));
        cur = parent.unwrap_or_default();
    }
    out.reverse();
    Ok(out)
}

/// Format d'export d'un document Google pour l'ouvrir avec une application du PC.
pub fn export_format(mime: &str) -> Option<(&'static str, &'static str)> {
    Some(match mime {
        "application/vnd.google-apps.document" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
        "application/vnd.google-apps.spreadsheet" => ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xlsx"),
        "application/vnd.google-apps.presentation" => ("application/vnd.openxmlformats-officedocument.presentationml.presentation", "pptx"),
        "application/vnd.google-apps.drawing" => ("image/png", "png"),
        _ => return None,
    })
}

/// Contenu du fichier (export Office pour les documents Google).
pub fn download(s: &Session, f: &DriveFile) -> Result<(Vec<u8>, String), String> {
    if f.is_google() {
        let (mime, ext) = export_format(&f.mime).ok_or("Ce type de document s'ouvre uniquement dans le navigateur")?;
        let data = s.get_bytes(&format!("{API}/files/{}/export", f.id), &[("mimeType", mime)], MAX_DOWNLOAD)?;
        Ok((data, format!("{}.{ext}", f.name)))
    } else {
        let data = s.get_bytes(&format!("{API}/files/{}", f.id), &[("alt", "media"), ("supportsAllDrives", "true")], MAX_DOWNLOAD)?;
        Ok((data, f.name.clone()))
    }
}

/// Miniature (lien temporaire de Drive, demandé avec le jeton).
pub fn thumbnail(s: &Session, url: &str) -> Result<Vec<u8>, String> {
    // Taille plus grande que la valeur par défaut (=s220).
    thumbnail_sized(s, url, 600)
}

/// Aperçu de `side` pixels (côté le plus long) : sert aussi à la visionneuse (version nette).
pub fn thumbnail_sized(s: &Session, url: &str, side: u32) -> Result<Vec<u8>, String> {
    let u = match url.rfind("=s") {
        Some(i) => format!("{}=s{side}", &url[..i]),
        None => url.to_string(),
    };
    s.get_bytes(&u, &[], 8 * 1024 * 1024)
}

/// Libellé du type de fichier.
pub fn kind_label(f: &DriveFile) -> &'static str {
    match f.mime.as_str() {
        FOLDER => "Dossier",
        "application/vnd.google-apps.document" => "Google Docs",
        "application/vnd.google-apps.spreadsheet" => "Google Sheets",
        "application/vnd.google-apps.presentation" => "Google Slides",
        "application/vnd.google-apps.form" => "Google Forms",
        "application/vnd.google-apps.drawing" => "Google Drawings",
        "application/vnd.google-apps.shortcut" => "Raccourci",
        "application/pdf" => "PDF",
        m if m.starts_with("image/") => "Image",
        m if m.starts_with("video/") => "Vidéo",
        m if m.starts_with("audio/") => "Audio",
        m if m.contains("zip") || m.contains("compressed") => "Archive",
        m if m.contains("word") => "Word",
        m if m.contains("sheet") || m.contains("excel") => "Excel",
        m if m.contains("presentation") || m.contains("powerpoint") => "PowerPoint",
        _ => "Fichier",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_file() {
        let f = parse(&json!({
            "id": "1", "name": "Brief.pdf", "mimeType": "application/pdf",
            "modifiedTime": "2026-10-20T10:00:00.000Z", "size": "2048",
            "owners": [{ "displayName": "Léa" }], "parents": ["p"]
        }));
        assert_eq!((f.size, f.owner.as_str(), f.extension().as_str()), (Some(2048), "Léa", "pdf"));
        assert_eq!(kind_label(&f), "PDF");
        assert!(export_format("application/vnd.google-apps.form").is_none());
        assert_eq!(escape("l'été"), "l\\'été");
    }
}
