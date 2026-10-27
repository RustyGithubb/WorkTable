//! Rédaction des messages sortants : contenu mis en forme (taille, alignement, gras),
//! images intégrées dans le texte, pièces jointes, puis génération MIME.
//!
//! Le gras est noté `**ainsi**` dans le texte (affiché en gras dans l'éditeur), un lien
//! `[texte](https://adresse)` (affiché souligné, l'adresse en petit).

use std::sync::Arc;

use lettre::message::header::ContentType;
use lettre::message::{Attachment as MimeAttachment, Mailbox, Mailboxes, Message, MultiPart, SinglePart};

use serde::{Deserialize, Serialize};

use super::FullMessage;

/// Gmail refuse les messages de plus de 25 Mo (encodage compris).
const MAX_TOTAL: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    New,
    Reply,
    Forward,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Size {
    Small,
    Normal,
    Large,
    Title,
}

impl Size {
    pub const ALL: [Size; 4] = [Size::Small, Size::Normal, Size::Large, Size::Title];
    pub fn px(self) -> f32 {
        match self {
            Size::Small => 12.0,
            Size::Normal => 14.0,
            Size::Large => 18.0,
            Size::Title => 24.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Size::Small => t!("Petit", "Small"),
            Size::Normal => "Normal",
            Size::Large => t!("Grand", "Large"),
            Size::Title => t!("Titre", "Heading"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub const ALL: [Align; 4] = [Align::Left, Align::Center, Align::Right, Align::Justify];
    fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Align::Left => t!("Aligner à gauche", "Align left"),
            Align::Center => t!("Centrer", "Centre"),
            Align::Right => t!("Aligner à droite", "Align right"),
            Align::Justify => t!("Justifier", "Justify"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub id: u64,
    pub text: String,
    pub size: Size,
    pub align: Align,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InlineImage {
    pub id: u64,
    pub name: String,
    pub mime: String,
    #[serde(with = "b64")]
    pub data: Arc<Vec<u8>>,
    pub cid: String,
    /// Image décodée pour l'aperçu dans l'éditeur (recalculée à la relecture d'un brouillon).
    #[serde(skip)]
    pub preview: Option<Arc<eframe::egui::ColorImage>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Block {
    Para(Paragraph),
    Image(InlineImage),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FileData {
    pub name: String,
    pub mime: String,
    #[serde(with = "b64")]
    pub data: Arc<Vec<u8>>,
}

/// Contenu binaire en base64 dans les brouillons enregistrés.
mod b64 {
    use std::sync::Arc;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(data.as_slice()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let text = String::deserialize(d)?;
        STANDARD.decode(text).map(Arc::new).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Draft {
    pub mode: Mode,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub blocks: Vec<Block>,
    pub files: Vec<FileData>,
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    /// Signature HTML (insérée comme dans Gmail), et paragraphe cité avant lequel la placer.
    pub signature: Option<String>,
    quote_id: Option<u64>,
    next_id: u64,
    /// Brouillon (comme dans Gmail) : identifiant local, brouillon Gmail correspondant, dernier
    /// enregistrement (ms).
    #[serde(default)]
    pub local_id: u64,
    #[serde(default)]
    pub gmail_id: Option<String>,
    /// Message Gmail du brouillon (change à chaque enregistrement) : le retrouver dans « Brouillons ».
    #[serde(default)]
    pub gmail_message_id: Option<String>,
    #[serde(default)]
    pub saved_at: Option<i64>,
}

/// Identifiant local unique d'un brouillon (horodatage + compteur).
fn next_local_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let base = chrono::Utc::now().timestamp_millis() as u64 * 1000;
    base + N.fetch_add(1, Ordering::Relaxed) % 1000
}

impl Draft {
    fn empty(mode: Mode) -> Self {
        let mut d = Draft {
            mode,
            to: String::new(),
            cc: String::new(),
            subject: String::new(),
            blocks: Vec::new(),
            files: Vec::new(),
            thread_id: None,
            in_reply_to: None,
            references: None,
            signature: None,
            quote_id: None,
            next_id: 0,
            local_id: next_local_id(),
            gmail_id: None,
            gmail_message_id: None,
            saved_at: None,
        };
        let p = d.paragraph(String::new());
        d.blocks.push(Block::Para(p));
        d
    }

    pub fn new() -> Self {
        Self::empty(Mode::New)
    }

    pub fn reply(m: &FullMessage) -> Self {
        let s = &m.summary;
        let mut d = Self::empty(Mode::Reply);
        d.to = if m.reply_to.is_empty() { s.from.clone() } else { m.reply_to.clone() };
        d.subject = prefixed("Re:", &s.subject);
        d.thread_id = Some(s.thread_id.clone());
        d.in_reply_to = (!m.message_id.is_empty()).then(|| m.message_id.clone());
        let refs = format!("{} {}", m.references, m.message_id).trim().to_string();
        d.references = (!refs.is_empty()).then_some(refs);
        let quoted = d.paragraph(tf!("{} a écrit :\n{}", "{} wrote:\n{}", s.from, quote(&m.body)));
        d.quote_id = Some(quoted.id);
        d.blocks.push(Block::Para(quoted));
        d
    }

    /// Répondre à tous : l'expéditeur en « À », les autres destinataires en « Cc » (sans soi-même).
    pub fn reply_all(m: &FullMessage, own: &str) -> Self {
        let mut d = Self::reply(m);
        let own = own.to_lowercase();
        let main = address_of(&d.to);
        let mut seen = vec![main.clone(), own.clone()];
        let mut cc = Vec::new();
        for a in split_addresses(&format!("{},{}", m.to, m.cc)) {
            let e = address_of(&a);
            if e.is_empty() || seen.contains(&e) {
                continue;
            }
            seen.push(e);
            cc.push(a);
        }
        d.cc = cc.join(", ");
        d
    }

    pub fn forward(m: &FullMessage, date: &str) -> Self {
        let s = &m.summary;
        let mut d = Self::empty(Mode::Forward);
        d.subject = prefixed("Fwd:", &s.subject);
        let fwd = d.paragraph(tf!(
            "---------- Message transféré ----------\nDe : {}\nDate : {}\nObjet : {}\nÀ : {}\n\n{}",
            "---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\nTo: {}\n\n{}",
            s.from,
            date,
            s.subject,
            m.to,
            m.body
        ));
        d.quote_id = Some(fwd.id);
        d.blocks.push(Block::Para(fwd));
        d
    }

    pub fn paragraph(&mut self, text: String) -> Paragraph {
        self.next_id += 1;
        Paragraph { id: self.next_id, text, size: Size::Normal, align: Align::Left }
    }

    /// Insère une image à la position `char_pos` du bloc `index` (le paragraphe est coupé en deux).
    /// Renvoie l'index du paragraphe qui suit l'image.
    pub fn insert_image(&mut self, index: usize, char_pos: usize, name: String, mime: String, data: Vec<u8>, preview: Option<Arc<eframe::egui::ColorImage>>) -> usize {
        self.next_id += 1;
        let img = InlineImage { id: self.next_id, cid: format!("img{}.{}@worktable", self.next_id, std::process::id()), name, mime, data: Arc::new(data), preview };
        let index = index.min(self.blocks.len().saturating_sub(1));
        match self.blocks.get_mut(index) {
            Some(Block::Para(p)) => {
                let byte = char_to_byte(&p.text, char_pos);
                let tail = p.text.split_off(byte);
                let (size, align) = (p.size, p.align);
                let mut after = self.paragraph(tail);
                after.size = size;
                after.align = align;
                self.blocks.insert(index + 1, Block::Image(img));
                self.blocks.insert(index + 2, Block::Para(after));
                index + 2
            }
            _ => {
                self.blocks.insert(index + 1, Block::Image(img));
                let p = self.paragraph(String::new());
                self.blocks.insert(index + 2, Block::Para(p));
                index + 2
            }
        }
    }

    pub fn total_size(&self) -> usize {
        self.files.iter().map(|f| f.data.len()).sum::<usize>() + self.blocks.iter().map(|b| if let Block::Image(i) = b { i.data.len() } else { 0 }).sum::<usize>()
    }

    pub fn plain_text(&self) -> String {
        let sig = self.signature.as_deref().map(|h| format!("\n-- \n{}", super::html::to_text(h).trim_end()));
        let mut out: Vec<String> = Vec::new();
        let mut sig_done = false;
        for b in &self.blocks {
            if let (Block::Para(p), Some(sig)) = (b, &sig)
                && Some(p.id) == self.quote_id
                && !sig_done
            {
                out.push(sig.clone());
                sig_done = true;
            }
            out.push(match b {
                Block::Para(p) => strip_bold(&p.text),
                Block::Image(i) => tf!("[image : {}]", "[image: {}]", i.name),
            });
        }
        if let (Some(sig), false) = (sig, sig_done) {
            out.push(sig);
        }
        out.join("\n")
    }

    /// Bloc signature au format Gmail (séparateur « -- » puis `gmail_signature`).
    fn signature_html(&self) -> Option<String> {
        self.signature.as_ref().map(|sig| format!("<br clear=\"all\"><div><br></div><span class=\"gmail_signature_prefix\">-- </span><br>{sig}"))
    }

    pub fn html(&self) -> String {
        let mut out = String::from("<div style=\"font-family:Arial,Helvetica,sans-serif;font-size:14px\">");
        let mut last_align = Align::Left;
        let sig = self.signature_html();
        let mut sig_done = false;
        for b in &self.blocks {
            if let (Block::Para(p), Some(sig)) = (b, &sig) {
                // Réponse / transfert : la signature se place au-dessus du message cité.
                if Some(p.id) == self.quote_id && !sig_done {
                    out.push_str(sig);
                    out.push_str("<br>");
                    sig_done = true;
                }
            }
            match b {
                Block::Para(p) => {
                    last_align = p.align;
                    let inner = if p.text.is_empty() { "<br>".to_string() } else { para_html(&p.text) };
                    out.push_str(&format!("<p style=\"margin:0 0 10px;text-align:{};font-size:{}px\">{inner}</p>", p.align.css(), p.size.px()));
                }
                Block::Image(i) => out.push_str(&format!(
                    "<p style=\"margin:0 0 10px;text-align:{}\"><img src=\"cid:{}\" alt=\"{}\" style=\"max-width:100%\"></p>",
                    last_align.css(),
                    i.cid,
                    escape(&i.name)
                )),
            }
        }
        if let (Some(sig), false) = (sig, sig_done) {
            out.push_str(&sig);
        }
        out.push_str("</div>");
        out
    }

    /// Message MIME prêt à être envoyé par l'API Gmail.
    pub fn to_mime(&self, from: &str) -> Result<Vec<u8>, String> {
        if self.total_size() > MAX_TOTAL {
            return Err(t!("Le message dépasse 25 Mo (limite Gmail). Retire des pièces jointes.", "The message exceeds 25 MB (Gmail limit). Remove some attachments.").into());
        }
        let from: Mailbox = from.parse().map_err(|e| tf!("Expéditeur invalide : {e}", "Invalid sender: {e}"))?;
        let to = parse_list(&self.to).map_err(|e| tf!("Destinataire invalide : {e}", "Invalid recipient: {e}"))?;
        if to.iter().next().is_none() {
            return Err(t!("Ajoute au moins un destinataire", "Add at least one recipient").into());
        }

        let mut b = Message::builder().from(from).subject(self.subject.clone());
        for m in to {
            b = b.to(m);
        }
        for m in parse_list(&self.cc).map_err(|e| tf!("Cc invalide : {e}", "Invalid Cc: {e}"))? {
            b = b.cc(m);
        }
        if let Some(id) = &self.in_reply_to {
            b = b.in_reply_to(id.clone());
        }
        if let Some(r) = &self.references {
            b = b.references(r.clone());
        }

        // texte + HTML
        let mut body = MultiPart::alternative_plain_html(self.plain_text(), self.html());
        // + images intégrées
        let images: Vec<&InlineImage> = self.blocks.iter().filter_map(|b| if let Block::Image(i) = b { Some(i) } else { None }).collect();
        if !images.is_empty() {
            let mut related = MultiPart::related().multipart(body);
            for i in images {
                related = related.singlepart(MimeAttachment::new_inline(i.cid.clone()).body(i.data.to_vec(), content_type(&i.mime)));
            }
            body = related;
        }
        // + pièces jointes
        if !self.files.is_empty() {
            let mut mixed = MultiPart::mixed().multipart(body);
            for f in &self.files {
                mixed = mixed.singlepart(attachment_part(f));
            }
            body = mixed;
        }

        let msg = b.multipart(body).map_err(|e| e.to_string())?;
        Ok(msg.formatted())
    }
}

fn attachment_part(f: &FileData) -> SinglePart {
    MimeAttachment::new(f.name.clone()).body(f.data.to_vec(), content_type(&f.mime))
}

fn content_type(mime: &str) -> ContentType {
    ContentType::parse(mime).unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap())
}

/// Type MIME d'après l'extension du fichier.
pub fn mime_for(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("zip") => "application/zip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

// --- Gras ------------------------------------------------------------------

/// Découpe `texte **gras** texte` en segments (texte, gras ?, est un marqueur ?).
pub fn bold_spans(text: &str) -> Vec<(&str, bool, bool)> {
    let mut out = Vec::new();
    let mut bold = false;
    let mut rest = text;
    // Un `**` n'ouvre du gras que s'il est refermé plus loin.
    while let Some(i) = rest.find("**") {
        let closes = bold || rest[i + 2..].contains("**");
        if !closes {
            break;
        }
        if i > 0 {
            out.push((&rest[..i], bold, false));
        }
        out.push((&rest[i..i + 2], false, true));
        bold = !bold;
        rest = &rest[i + 2..];
    }
    if !rest.is_empty() {
        out.push((rest, bold, false));
    }
    out
}

// --- Liens -----------------------------------------------------------------

/// Morceau de texte mis en forme : gras, lien (adresse), ou marqueur (`**`, `[`, `](adresse)`).
#[derive(Debug, PartialEq)]
pub struct Span<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub link: Option<&'a str>,
    pub marker: bool,
}

/// Liens `[texte](adresse)` d'un paragraphe : (début, fin, début du texte, fin du texte, adresse),
/// en octets.
fn links(text: &str) -> Vec<(usize, usize, usize, usize, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find('[').map(|i| i + from) {
        let Some(j) = text[i + 1..].find("](").map(|j| j + i + 1) else { break };
        let Some(k) = text[j + 2..].find(')').map(|k| k + j + 2) else { break };
        let (label, url) = (&text[i + 1..j], &text[j + 2..k]);
        if !label.is_empty() && !label.contains(['\n', '[']) && !url.is_empty() && !url.contains(char::is_whitespace) {
            out.push((i, k + 1, i + 1, j, url));
            from = k + 1;
        } else {
            from = i + 1;
        }
    }
    out
}

/// Découpe un paragraphe en morceaux (gras, liens, marqueurs), dans l'ordre.
pub fn spans(text: &str) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for (start, end, ls, le, url) in links(text) {
        if start > pos {
            for (seg, bold, marker) in bold_spans(&text[pos..start]) {
                out.push(Span { text: seg, bold, link: None, marker });
            }
        }
        out.push(Span { text: &text[start..ls], bold: false, link: None, marker: true });
        for (seg, bold, marker) in bold_spans(&text[ls..le]) {
            out.push(Span { text: seg, bold, link: Some(url), marker });
        }
        out.push(Span { text: &text[le..end], bold: false, link: None, marker: true });
        pos = end;
    }
    if pos < text.len() {
        for (seg, bold, marker) in bold_spans(&text[pos..]) {
            out.push(Span { text: seg, bold, link: None, marker });
        }
    }
    out
}

/// Lien sous le curseur (caractère `at`) : (début, fin en caractères, texte, adresse).
pub fn link_at(text: &str, at: usize) -> Option<(usize, usize, String, String)> {
    let c = |b: usize| text[..b].chars().count();
    links(text).into_iter().find(|(s, e, ..)| c(*s) <= at && at <= c(*e)).map(|(s, e, ls, le, url)| (c(s), c(e), text[ls..le].to_string(), url.to_string()))
}

/// Remplace les caractères [a, b) par un lien `[texte](adresse)` ; renvoie la position du curseur
/// après le lien.
pub fn set_link(text: &mut String, a: usize, b: usize, label: &str, url: &str) -> usize {
    let (a, b) = (a.min(b), a.max(b));
    let label = if label.trim().is_empty() { url } else { label.trim() };
    let label = label.replace(['[', ']'], "");
    let link = format!("[{label}]({})", normalize_url(url));
    let (ba, bb) = (char_to_byte(text, a), char_to_byte(text, b));
    text.replace_range(ba..bb, &link);
    a + link.chars().count()
}

/// Retire le lien [a, b) (caractères) en gardant son texte ; renvoie la fin du texte.
pub fn remove_link(text: &mut String, a: usize, b: usize, label: &str) -> usize {
    let (ba, bb) = (char_to_byte(text, a), char_to_byte(text, b));
    text.replace_range(ba..bb, label);
    a + label.chars().count()
}

/// « exemple.fr » → « https://exemple.fr », « jean@x.fr » → « mailto:jean@x.fr ».
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    if u.contains("://") || u.starts_with("mailto:") || u.starts_with("tel:") {
        u.to_string()
    } else if u.contains('@') && !u.contains('/') {
        format!("mailto:{u}")
    } else {
        format!("https://{u}")
    }
}

fn strip_bold(text: &str) -> String {
    let mut out = String::new();
    for sp in spans(text) {
        if sp.marker {
            // Fin d'un lien : l'adresse entre parenthèses, lisible en texte brut.
            if let Some(url) = sp.text.strip_prefix("](").and_then(|u| u.strip_suffix(')')) {
                out.push_str(&format!(" ({url})"));
            }
            continue;
        }
        out.push_str(sp.text);
    }
    out
}

fn para_html(text: &str) -> String {
    let mut out = String::new();
    let mut open: Option<&str> = None;
    for sp in spans(text) {
        if sp.link != open {
            if open.is_some() {
                out.push_str("</a>");
            }
            if let Some(u) = sp.link {
                out.push_str(&format!("<a href=\"{}\">", escape(u)));
            }
            open = sp.link;
        }
        if sp.marker {
            continue;
        }
        let t = escape(sp.text).replace('\n', "<br>");
        out.push_str(&if sp.bold { format!("<b>{t}</b>") } else { t });
    }
    if open.is_some() {
        out.push_str("</a>");
    }
    out
}

/// Met en gras (ou retire le gras de) la sélection [a, b) en caractères.
/// Renvoie la nouvelle sélection.
pub fn toggle_bold(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    let (a, b) = (a.min(b), a.max(b));
    let chars: Vec<char> = text.chars().collect();
    let around = a >= 2 && b + 2 <= chars.len() && chars[a - 2..a] == ['*', '*'] && chars[b..b + 2] == ['*', '*'];
    let (ba, bb) = (char_to_byte(text, a), char_to_byte(text, b));
    if around {
        text.replace_range(bb..bb + 2, "");
        text.replace_range(ba - 2..ba, "");
        (a - 2, b - 2)
    } else {
        text.insert_str(bb, "**");
        text.insert_str(ba, "**");
        (a + 2, b + 2)
    }
}

pub fn char_to_byte(s: &str, char_pos: usize) -> usize {
    s.char_indices().nth(char_pos).map(|(i, _)| i).unwrap_or(s.len())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Liste de destinataires tolérante : virgule finale (ajoutée par les suggestions), noms avec
/// accents, points, parenthèses ou virgules entre guillemets. Chaque entrée est « Nom <adresse> »
/// ou « adresse » ; seule l'adresse doit être valide (le nom est remis en forme à l'envoi).
fn parse_list(s: &str) -> Result<Mailboxes, String> {
    let mut out = Mailboxes::new();
    for entry in split_addresses(s) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (name, addr) = match (entry.rfind('<'), entry.rfind('>')) {
            (Some(a), Some(b)) if a < b => (entry[..a].trim(), entry[a + 1..b].trim()),
            _ => ("", entry),
        };
        let name = name.trim_matches(['"', '\'', ' ']).replace("\\\"", "\"");
        let addr = addr.trim_matches(['"', '\'', ' ']);
        let address = addr.parse::<lettre::Address>().map_err(|_| format!("« {entry} »"))?;
        out.push(Mailbox::new((!name.is_empty()).then_some(name), address));
    }
    Ok(out)
}

fn prefixed(prefix: &str, subject: &str) -> String {
    if subject.to_lowercase().starts_with(&prefix.to_lowercase()) { subject.to_string() } else { format!("{prefix} {subject}") }
}

fn quote(body: &str) -> String {
    body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::{FullMessage, Summary};

    #[test]
    fn links_in_paragraphs() {
        let t = "Voir [le **site**](https://x.fr) et [doc](y.fr";
        assert_eq!(para_html(t), "Voir <a href=\"https://x.fr\">le <b>site</b></a> et [doc](y.fr");
        assert_eq!(strip_bold(t), "Voir le site (https://x.fr) et [doc](y.fr");
        let mut s = String::from("Clique ici pour voir");
        let end = set_link(&mut s, 7, 10, "ici", "exemple.fr");
        assert_eq!(s, "Clique [ici](https://exemple.fr) pour voir");
        assert_eq!(end, 32);
        assert_eq!(link_at(&s, 9), Some((7, 32, "ici".into(), "https://exemple.fr".into())));
        assert_eq!(link_at(&s, 2), None);
        let end = remove_link(&mut s, 7, 32, "ici");
        assert_eq!((s.as_str(), end), ("Clique ici pour voir", 10));
        assert_eq!(normalize_url("jean@x.fr"), "mailto:jean@x.fr");
    }

    #[test]
    fn recipients_from_suggestions_are_accepted() {
        let list = |s: &str| parse_list(s).map(|m| m.into_iter().map(|x| x.email.to_string()).collect::<Vec<_>>());
        // Suggestion choisie : « Nom <adresse>, » (virgule finale).
        assert_eq!(list("Hugo Bernard <hugo@gmail.com>, "), Ok(vec!["hugo@gmail.com".to_string()]));
        assert_eq!(list("Jean (Studio) <j@d.fr>; Léa Martin <lea@studio.fr>,"), Ok(vec!["j@d.fr".to_string(), "lea@studio.fr".to_string()]));
        assert_eq!(list("\"Dupont, Jean\" <j@d.fr>, a@b.fr, "), Ok(vec!["j@d.fr".to_string(), "a@b.fr".to_string()]));
        assert!(list("Hugo <pas-une-adresse>").is_err());
        assert_eq!(list("  "), Ok(vec![]));
    }

    fn sample() -> FullMessage {
        FullMessage {
            summary: Summary {
                id: "1".into(),
                thread_id: "t1".into(),
                from: "Léa Martin <lea@studio.fr>".into(),
                subject: "Retours août".into(),
                snippet: String::new(),
                date: 0,
                labels: vec![],
                ..Default::default()
            },
            to: "moi@gmail.com".into(),
            cc: String::new(),
            reply_to: String::new(),
            message_id: "<abc@mail.gmail.com>".into(),
            references: String::new(),
            body: "Ligne 1\nLigne 2".into(),
            blocks: vec![],
            attachments: vec![],
        }
    }

    #[test]
    fn reply_is_threaded() {
        let d = Draft::reply(&sample());
        assert_eq!(d.subject, "Re: Retours août");
        assert_eq!(d.thread_id.as_deref(), Some("t1"));
        let mime = String::from_utf8_lossy(&d.to_mime("moi@gmail.com").unwrap()).into_owned();
        assert!(mime.contains("In-Reply-To: <abc@mail.gmail.com>"));
        assert!(mime.contains("lea@studio.fr"));
        assert!(mime.contains("multipart/alternative"));
    }

    #[test]
    fn forward_needs_recipient() {
        let d = Draft::forward(&sample(), "hier");
        assert_eq!(d.subject, "Fwd: Retours août");
        assert!(d.to_mime("moi@gmail.com").is_err());
    }

    #[test]
    fn signature_like_gmail() {
        let sig = r#"<div class="gmail_signature"><b>Dorian</b> — ARTFX</div>"#;
        let mut d = Draft::new();
        d.signature = Some(sig.into());
        if let Block::Para(p) = &mut d.blocks[0] {
            p.text = "Bonjour".into();
        }
        let html = d.html();
        let body = html.find("Bonjour").unwrap();
        let s = html.find("gmail_signature_prefix").unwrap();
        assert!(body < s && html.contains(sig));
        assert!(d.plain_text().ends_with("-- \nDorian — ARTFX"), "{}", d.plain_text());

        // Réponse : signature au-dessus du message cité.
        let mut r = Draft::reply(&sample());
        r.signature = Some(sig.into());
        let html = r.html();
        assert!(html.find("ARTFX").unwrap() < html.find("a écrit").unwrap());
    }

    #[test]
    fn bold_toggle_and_html() {
        let mut t = String::from("Bonjour Dorian !");
        let sel = toggle_bold(&mut t, 8, 14);
        assert_eq!(t, "Bonjour **Dorian** !");
        assert_eq!(sel, (10, 16));
        assert_eq!(para_html(&t), "Bonjour <b>Dorian</b> !");
        assert_eq!(strip_bold(&t), "Bonjour Dorian !");
        let back = toggle_bold(&mut t, sel.0, sel.1);
        assert_eq!(t, "Bonjour Dorian !");
        assert_eq!(back, (8, 14));
        // Un ** isolé reste du texte.
        assert_eq!(para_html("2 ** 3"), "2 ** 3");
    }

    #[test]
    fn image_and_attachment_in_mime() {
        let mut d = Draft::new();
        d.to = "a@b.fr".into();
        d.subject = "Test".into();
        if let Block::Para(p) = &mut d.blocks[0] {
            p.text = "Avant après".into();
            p.align = Align::Center;
            p.size = Size::Large;
        }
        let next = d.insert_image(0, 6, "photo.png".into(), "image/png".into(), vec![1, 2, 3], None);
        assert_eq!(next, 2);
        d.files.push(FileData { name: "doc.pdf".into(), mime: "application/pdf".into(), data: Arc::new(vec![9; 10]) });

        let html = d.html();
        assert!(html.contains("text-align:center;font-size:18px\">Avant </p>"), "{html}");
        let Block::Image(img) = &d.blocks[1] else { panic!() };
        assert!(html.contains(&format!("cid:{}", img.cid)));

        let mime = String::from_utf8_lossy(&d.to_mime("moi@gmail.com").unwrap()).into_owned();
        assert!(mime.contains("multipart/mixed"));
        assert!(mime.contains("multipart/related"));
        assert!(mime.contains(&format!("Content-ID: <{}>", img.cid)));
        assert!(mime.contains("filename=\"doc.pdf\""));
    }
}

/// Découpe une liste d'adresses (virgules hors guillemets).
fn split_addresses(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' | ';' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// Adresse e-mail seule, en minuscules (« Jean <j@x.fr> » → « j@x.fr »).
fn address_of(s: &str) -> String {
    let e = match (s.rfind('<'), s.rfind('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    e.trim().trim_matches('"').to_lowercase()
}

#[cfg(test)]
mod reply_all_tests {
    use super::*;

    #[test]
    fn reply_all_skips_self_and_sender() {
        let m = FullMessage {
            summary: crate::google::Summary { from: "Ana <ana@x.fr>".into(), subject: "Point".into(), ..Default::default() },
            to: "Moi <moi@x.fr>, \"Dupont, Léo\" <leo@x.fr>".into(),
            cc: "ana@x.fr, zoe@x.fr".into(),
            ..Default::default()
        };
        let d = Draft::reply_all(&m, "MOI@x.fr");
        assert_eq!(d.to, "Ana <ana@x.fr>");
        assert_eq!(d.cc, "\"Dupont, Léo\" <leo@x.fr>, zoe@x.fr");
    }
}
