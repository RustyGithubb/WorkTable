//! Conversion du corps d'un mail en paragraphes mis en forme + images, dans l'ordre.
//!
//! Deux sources de mise en forme sont reconnues :
//! - le **HTML** produit par Gmail et les autres messageries : `<b>`/`<strong>`, `<i>`/`<em>`,
//!   `<u>`, `<s>`/`<strike>`/`<del>`, `<font size>`, `<h1>`…`<h6>`, `style="font-weight /
//!   font-style / text-decoration / font-size / text-align"`, `<a href>`, listes `<ul>/<ol>/<li>`,
//!   citations `<blockquote>`, `<code>/<pre>`, images `<img>` ;
//! - le **Markdown** écrit en texte brut : `**gras**`, `__gras__`, `*italique*`, `_italique_`,
//!   `~~barré~~`, `` `code` ``, titres `#`, citations `>`, puces `-`/`*`/`+`.

use scraper::{ElementRef, Html, Node};

// --- Modèle ------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ImageSrc {
    /// Image intégrée au mail (`cid:` → partie MIME avec ce Content-ID).
    Cid(String),
    /// Image hébergée (http/https).
    Url(String),
    /// Image encodée directement dans le HTML (`data:image/png;base64,…`).
    Data(Vec<u8>),
}

/// Dimension HTML/CSS : en pixels ou en pourcentage de la largeur disponible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    Px(f32),
    Percent(f32),
}

impl Dim {
    pub fn resolve(self, available: f32) -> f32 {
        match self {
            Dim::Px(v) => v,
            Dim::Percent(p) => available * p / 100.0,
        }
    }
}

fn parse_dim(v: &str) -> Option<Dim> {
    let v = v.trim().to_ascii_lowercase();
    let v = v.trim_end_matches("!important").trim();
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().filter(|x| *x > 0.0).map(Dim::Percent);
    }
    css_px(v)
        .or_else(|| v.parse::<f32>().ok())
        .filter(|x| *x > 0.0)
        .map(Dim::Px)
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    /// Facteur de taille (1.0 = taille normale).
    pub scale: f32,
    pub link: Option<String>,
    /// Lien vers un fichier à télécharger (affiché en bleu souligné).
    pub download: bool,
    /// Couleur imposée (`<font color>`, `style="color:…"`).
    pub color: Option<[u8; 3]>,
}

impl Default for Run {
    fn default() -> Self {
        Run {
            text: String::new(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            code: false,
            scale: 1.0,
            link: None,
            download: false,
            color: None,
        }
    }
}

impl Run {
    fn same_style(&self, o: &Run) -> bool {
        self.bold == o.bold
            && self.italic == o.italic
            && self.underline == o.underline
            && self.strike == o.strike
            && self.code == o.code
            && self.scale == o.scale
            && self.link == o.link
            && self.download == o.download
            && self.color == o.color
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Para {
    pub runs: Vec<Run>,
    pub align: Align,
    /// Profondeur de citation (barre verticale à gauche).
    pub quote: u8,
    /// Retrait (listes imbriquées).
    pub indent: u8,
}

impl Para {
    #[allow(dead_code)]
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    fn is_blank(&self) -> bool {
        self.runs.iter().all(|r| r.text.trim().is_empty())
    }
    fn push(&mut self, run: Run) {
        if run.text.is_empty() {
            return;
        }
        if let Some(last) = self.runs.last_mut() {
            if last.same_style(&run) {
                last.text.push_str(&run.text);
                return;
            }
        }
        self.runs.push(run);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BodyBlock {
    Para(Para),
    Image {
        src: ImageSrc,
        alt: String,
        /// Taille imposée par l'expéditeur (attributs `width`/`height` ou CSS en ligne).
        width: Option<Dim>,
        height: Option<Dim>,
        max_width: Option<Dim>,
        /// Alignement hérité (`text-align`, `align`, `<center>`, `margin:auto`, `float`).
        align: Align,
        /// Image cliquable (dans un lien `<a href>`).
        link: Option<String>,
    },
}

// --- HTML ----------------------------------------------------------------------

/// Texte brut d'un HTML (pour citer le message dans une réponse).
pub fn to_text(html: &str) -> String {
    html2text::config::plain_no_decorate()
        .link_footnotes(false)
        .no_table_borders()
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_else(|_| html.to_string())
}

#[derive(Clone, Default)]
struct Ctx {
    style: Run,
    align: Align,
    quote: u8,
    pre: bool,
}

struct Builder {
    out: Vec<BodyBlock>,
    cur: Para,
    /// Listes ouvertes : (ordonnée ?, compteur).
    lists: Vec<(bool, u32)>,
}

impl Builder {
    fn flush(&mut self, ctx: &Ctx) {
        let mut p = std::mem::take(&mut self.cur);
        // Nettoyage des espaces en fin de paragraphe.
        while let Some(last) = p.runs.last_mut() {
            let t = last.text.trim_end_matches([' ', '\t']).to_string();
            if t.is_empty() {
                p.runs.pop();
            } else {
                last.text = t;
                break;
            }
        }
        let blank = p.is_blank();
        // Une seule ligne vide d'affilée au maximum.
        let prev_blank = matches!(self.out.last(), Some(BodyBlock::Para(q)) if q.is_blank()) || self.out.is_empty();
        if blank && (prev_blank || !p.runs.iter().any(|r| r.text.contains('\n'))) {
            self.cur = Para { align: ctx.align, quote: ctx.quote, indent: self.lists.len() as u8, ..Default::default() };
            return;
        }
        if blank {
            p.runs.clear();
        }
        self.out.push(BodyBlock::Para(p));
        self.cur = Para { align: ctx.align, quote: ctx.quote, indent: self.lists.len() as u8, ..Default::default() };
    }

    fn start(&mut self, ctx: &Ctx) {
        if self.cur.runs.is_empty() {
            self.cur.align = ctx.align;
            self.cur.quote = ctx.quote;
            self.cur.indent = self.lists.len() as u8;
        }
    }

    fn text(&mut self, ctx: &Ctx, raw: &str) {
        let text = if ctx.pre {
            raw.to_string()
        } else {
            let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            let lead = raw.starts_with(char::is_whitespace) && !collapsed.is_empty();
            let trail = raw.ends_with(char::is_whitespace) && !raw.trim().is_empty();
            let at_line_start = self.cur.runs.last().is_none_or(|r| r.text.ends_with('\n') || r.text.ends_with(' '));
            let mut s = String::new();
            if (lead || (collapsed.is_empty() && !raw.is_empty())) && !at_line_start {
                s.push(' ');
            }
            s.push_str(&collapsed);
            if trail && !collapsed.is_empty() {
                s.push(' ');
            }
            s
        };
        if text.is_empty() {
            return;
        }
        self.start(ctx);
        self.cur.push(Run { text, ..ctx.style.clone() });
    }
}

pub fn from_html(html: &str) -> Vec<BodyBlock> {
    let doc = Html::parse_document(html);
    let mut b = Builder { out: Vec::new(), cur: Para::default(), lists: Vec::new() };
    let ctx = Ctx::default();
    walk(&mut b, doc.root_element(), &ctx);
    b.flush(&ctx);
    // Retire les lignes vides en fin de message.
    while matches!(b.out.last(), Some(BodyBlock::Para(p)) if p.is_blank()) {
        b.out.pop();
    }
    apply_markdown(b.out)
}

const BLOCKS: &[&str] = &[
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "blockquote", "tr", "table", "ul", "ol",
    "pre", "center", "section", "article", "header", "footer", "hr", "dl", "dt", "dd", "address",
    "body", "tbody", "thead",
];

fn walk(b: &mut Builder, el: ElementRef, parent: &Ctx) {
    let name = el.value().name().to_ascii_lowercase();
    if matches!(name.as_str(), "style" | "script" | "head" | "title" | "meta" | "link" | "noscript" | "template") {
        return;
    }
    let mut ctx = parent.clone();
    apply_tag(&name, el, &mut ctx);
    if let Some(style) = el.value().attr("style") {
        apply_css(style, &mut ctx);
    }
    if let Some(a) = el.value().attr("align") {
        ctx.align = parse_align(a).unwrap_or(ctx.align);
    }

    let is_block = BLOCKS.contains(&name.as_str());
    if is_block {
        b.flush(parent);
    }

    match name.as_str() {
        "br" => {
            b.start(&ctx);
            b.cur.push(Run { text: "\n".into(), ..ctx.style.clone() });
            return;
        }
        "img" => {
            b.flush(&ctx);
            if let Some(img) = image_block(el, &ctx) {
                b.out.push(img);
            }
            return;
        }
        "hr" => return,
        "ul" | "ol" => b.lists.push((name == "ol", 0)),
        "li" => {
            let prefix = match b.lists.last_mut() {
                Some((true, n)) => {
                    *n += 1;
                    format!("{n}. ")
                }
                _ => "•  ".to_string(),
            };
            b.start(&ctx);
            b.cur.indent = b.lists.len().saturating_sub(1) as u8;
            b.cur.push(Run { text: prefix, ..Run::default() });
        }
        _ => {}
    }

    for child in el.children() {
        match child.value() {
            Node::Text(t) => b.text(&ctx, t),
            Node::Element(_) => {
                if let Some(e) = ElementRef::wrap(child) {
                    walk(b, e, &ctx);
                }
            }
            _ => {}
        }
    }
    if matches!(name.as_str(), "td" | "th") {
        b.text(&ctx, " ");
    }

    if matches!(name.as_str(), "ul" | "ol") {
        b.lists.pop();
    }
    if is_block {
        b.flush(&ctx);
    }
}

fn apply_tag(name: &str, el: ElementRef, ctx: &mut Ctx) {
    let s = &mut ctx.style;
    match name {
        "b" | "strong" | "th" => s.bold = true,
        "i" | "em" | "cite" | "var" | "dfn" => s.italic = true,
        "u" | "ins" => s.underline = true,
        "s" | "strike" | "del" => s.strike = true,
        "code" | "tt" | "kbd" | "samp" => s.code = true,
        "pre" => {
            s.code = true;
            ctx.pre = true;
        }
        "small" => s.scale *= 0.85,
        "big" => s.scale *= 1.2,
        "h1" => heading(s, 1.6),
        "h2" => heading(s, 1.4),
        "h3" => heading(s, 1.2),
        "h4" | "h5" | "h6" => heading(s, 1.05),
        "center" => ctx.align = Align::Center,
        "blockquote" => ctx.quote = ctx.quote.saturating_add(1),
        "a" => {
            if let Some(h) = el.value().attr("href").filter(|h| !h.starts_with('#')) {
                s.link = Some(h.to_string());
                s.underline = true;
            }
        }
        "font" => {
            if let Some(c) = el.value().attr("color").and_then(parse_color) {
                s.color = Some(c);
            }
            if let Some(n) = el.value().attr("size").and_then(|v| v.trim().parse::<i32>().ok()) {
                // Tailles HTML 1 à 7 (Gmail : 2 = petit, 3 = normal, 4 = grand, 6 = très grand).
                const SCALE: [f32; 7] = [0.7, 0.85, 1.0, 1.3, 1.6, 2.0, 2.6];
                s.scale = SCALE[(n.clamp(1, 7) - 1) as usize];
            }
        }
        _ => {}
    }
}

fn heading(s: &mut Run, scale: f32) {
    s.bold = true;
    s.scale = scale;
}

/// Styles CSS en ligne (seul le CSS en ligne est conservé par Gmail).
fn apply_css(style: &str, ctx: &mut Ctx) {
    for decl in style.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase());
        let v = v.trim_end_matches("!important").trim();
        let s = &mut ctx.style;
        match k.as_str() {
            "font-weight" => s.bold = v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => s.italic = v == "italic" || v == "oblique",
            "text-decoration" | "text-decoration-line" => {
                if v.contains("underline") {
                    s.underline = true;
                }
                if v.contains("line-through") {
                    s.strike = true;
                }
                if v == "none" {
                    s.underline = false;
                }
            }
            "font-size" => {
                if let Some(px) = css_px(v) {
                    s.scale = (px / 14.0).clamp(0.7, 2.6);
                }
            }
            "text-align" => ctx.align = parse_align(v).unwrap_or(ctx.align),
            "color" => s.color = parse_color(v).or(s.color),
            "font-family" if v.contains("monospace") || v.contains("courier") => s.code = true,
            _ => {}
        }
    }
}

/// Couleur CSS : `#rgb`, `#rrggbb`, `rgb(r, g, b)` ou nom courant.
pub fn parse_color(v: &str) -> Option<[u8; 3]> {
    let v = v.trim().to_ascii_lowercase();
    let v = v.trim_end_matches("!important").trim();
    if let Some(h) = v.strip_prefix('#') {
        let h: String = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect() } else { h.to_string() };
        if h.len() != 6 {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        return Some([p(0)?, p(2)?, p(4)?]);
    }
    if let Some(inner) = v.strip_prefix("rgb(").or_else(|| v.strip_prefix("rgba(")) {
        let n: Vec<u8> = inner.trim_end_matches(')').split(',').take(3).filter_map(|x| x.trim().parse::<f32>().ok()).map(|x| x.clamp(0.0, 255.0) as u8).collect();
        return (n.len() == 3).then(|| [n[0], n[1], n[2]]);
    }
    Some(match v {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "red" => [255, 0, 0],
        "maroon" => [128, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "navy" => [0, 0, 128],
        "orange" => [255, 165, 0],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        _ => return None,
    })
}

fn css_px(v: &str) -> Option<f32> {
    let num = |suffix: &str| v.strip_suffix(suffix).and_then(|n| n.trim().parse::<f32>().ok());
    if let Some(n) = num("px") {
        Some(n)
    } else if let Some(n) = num("pt") {
        Some(n * 4.0 / 3.0)
    } else if let Some(n) = num("em").or_else(|| num("rem")) {
        Some(n * 14.0)
    } else if let Some(n) = num("%") {
        Some(n / 100.0 * 14.0)
    } else {
        match v {
            "xx-small" => Some(9.0),
            "x-small" => Some(10.0),
            "small" | "smaller" => Some(12.0),
            "medium" => Some(14.0),
            "large" | "larger" => Some(18.0),
            "x-large" => Some(24.0),
            "xx-large" => Some(32.0),
            _ => None,
        }
    }
}

fn parse_align(v: &str) -> Option<Align> {
    match v.trim() {
        "left" | "start" => Some(Align::Left),
        "center" | "middle" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        "justify" => Some(Align::Justify),
        _ => None,
    }
}

fn image_block(el: ElementRef, ctx: &Ctx) -> Option<BodyBlock> {
    let v = el.value();
    let mut width = v.attr("width").and_then(parse_dim);
    let mut height = v.attr("height").and_then(parse_dim);
    let mut max_width = None;
    let mut align = ctx.align;
    if let Some(a) = v.attr("align") {
        // align="left|right" sur une image = flottante de ce côté.
        align = parse_align(a).unwrap_or(align);
    }
    // Le CSS en ligne l'emporte sur les attributs (comme dans Gmail).
    let mut margin_auto = (false, false);
    for decl in v.attr("style").unwrap_or("").split(';') {
        let Some((k, val)) = decl.split_once(':') else { continue };
        let (k, val) = (k.trim().to_ascii_lowercase(), val.trim().to_ascii_lowercase());
        match k.as_str() {
            "width" => width = parse_dim(&val).or(width),
            "height" => height = parse_dim(&val).or(height),
            "max-width" => max_width = parse_dim(&val),
            "float" if val == "right" => align = Align::Right,
            "float" if val == "left" => align = Align::Left,
            "margin" => {
                let parts: Vec<&str> = val.split_whitespace().collect();
                let horiz = match parts.len() {
                    1 => parts[0],
                    2 | 3 => parts[1],
                    _ => parts.get(1).copied().unwrap_or(""),
                };
                if horiz == "auto" {
                    margin_auto = (true, true);
                }
            }
            "margin-left" if val == "auto" => margin_auto.0 = true,
            "margin-right" if val == "auto" => margin_auto.1 = true,
            _ => {}
        }
    }
    if margin_auto == (true, true) {
        align = Align::Center;
    } else if margin_auto.0 {
        align = Align::Right;
    }

    let tiny = |d: Option<Dim>| matches!(d, Some(Dim::Px(x)) if x <= 2.0);
    if tiny(width) || tiny(height) {
        return None;
    }
    let src = parse_src(v.attr("src")?)?;
    Some(BodyBlock::Image {
        src,
        alt: v.attr("alt").unwrap_or_default().to_string(),
        width,
        height,
        max_width,
        align,
        link: ctx.style.link.clone(),
    })
}

fn parse_src(src: &str) -> Option<ImageSrc> {
    let s = src.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("cid:") {
        Some(ImageSrc::Cid(s[4..].trim_matches(['<', '>']).to_string()))
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(ImageSrc::Url(s.to_string()))
    } else if lower.starts_with("data:image/") {
        use base64::Engine;
        let (_, data) = s.split_once("base64,")?;
        base64::engine::general_purpose::STANDARD.decode(data.trim()).ok().map(ImageSrc::Data)
    } else {
        None
    }
}

// --- Markdown (texte brut) ---------------------------------------------------

/// Mail en texte brut : chaque groupe de lignes devient un paragraphe ;
/// titres, citations, puces et styles Markdown sont interprétés.
pub fn from_plain(text: &str) -> Vec<BodyBlock> {
    let mut out = Vec::new();
    let mut cur: Option<Para> = None;
    let flush = |cur: &mut Option<Para>, out: &mut Vec<BodyBlock>| {
        if let Some(p) = cur.take() {
            out.push(BodyBlock::Para(p));
        }
    };

    for line in text.replace("\r\n", "\n").split('\n') {
        // Citations « > » (éventuellement imbriquées).
        let mut rest = line;
        let mut quote = 0u8;
        while let Some(r) = rest.strip_prefix('>') {
            quote += 1;
            rest = r.strip_prefix(' ').unwrap_or(r);
        }
        let trimmed = rest.trim_start();

        if trimmed.is_empty() {
            flush(&mut cur, &mut out);
            if !matches!(out.last(), Some(BodyBlock::Para(p)) if p.runs.is_empty()) && !out.is_empty() {
                out.push(BodyBlock::Para(Para { quote, ..Default::default() }));
            }
            continue;
        }

        // Titres « # », « ## », « ### ».
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            flush(&mut cur, &mut out);
            let scale = [1.6, 1.4, 1.2, 1.05, 1.0, 1.0][hashes - 1];
            let run = Run { text: trimmed[hashes + 1..].trim().to_string(), bold: true, scale, ..Default::default() };
            out.push(BodyBlock::Para(Para { runs: vec![run], quote, ..Default::default() }));
            continue;
        }
        // Ligne de séparation.
        if trimmed.len() >= 3 && (trimmed.chars().all(|c| c == '-') || trimmed.chars().all(|c| c == '*') || trimmed.chars().all(|c| c == '_')) {
            flush(&mut cur, &mut out);
            continue;
        }
        // Puces.
        let indent = (rest.len() - trimmed.len()) / 2;
        let line_text = match trimmed.get(..2) {
            Some("- " | "* " | "+ ") => format!("{}•  {}", "    ".repeat(indent), &trimmed[2..]),
            _ => rest.to_string(),
        };

        match &mut cur {
            Some(p) if p.quote == quote => p.push(Run { text: format!("\n{line_text}"), ..Default::default() }),
            _ => {
                flush(&mut cur, &mut out);
                let mut p = Para { quote, ..Default::default() };
                p.push(Run { text: line_text, ..Default::default() });
                cur = Some(p);
            }
        }
    }
    flush(&mut cur, &mut out);
    while matches!(out.last(), Some(BodyBlock::Para(p)) if p.runs.is_empty()) {
        out.pop();
    }
    apply_markdown(out)
}

/// Applique le Markdown en ligne (`**gras**`, `*italique*`, `~~barré~~`, `` `code` ``)
/// au texte de chaque paragraphe.
fn apply_markdown(blocks: Vec<BodyBlock>) -> Vec<BodyBlock> {
    blocks
        .into_iter()
        .map(|b| match b {
            BodyBlock::Para(mut p) => {
                let mut runs = Vec::new();
                for r in p.runs {
                    if r.code || r.link.is_some() {
                        runs.push(r);
                    } else {
                        let mut md = Vec::new();
                        inline_md(&r.text, &r, &mut md);
                        for m in md {
                            linkify(m, &mut runs);
                        }
                    }
                }
                p.runs = Vec::new();
                for mut r in runs {
                    if let Some(url) = &r.link {
                        r.download = is_download(url, &r.text);
                    }
                    p.push(r);
                }
                BodyBlock::Para(p)
            }
            other => other,
        })
        .collect()
}

/// Transforme les adresses web écrites en texte (http://, https://, www.) en liens.
pub fn linkify(run: Run, out: &mut Vec<Run>) {
    if run.code || run.link.is_some() {
        out.push(run);
        return;
    }
    let text = run.text.as_str();
    let lower = text.to_ascii_lowercase();
    let mut pos = 0;
    let mut last = 0;
    while pos < text.len() {
        let rest = &lower[pos..];
        let starts = rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.");
        let boundary = pos == 0 || !text[..pos].ends_with(|c: char| c.is_alphanumeric() || c == '/' || c == '@');
        if starts && boundary {
            let len = text[pos..]
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '«' | '»'))
                .unwrap_or(text.len() - pos);
            let mut url = &text[pos..pos + len];
            // Ponctuation finale exclue (« voir https://x.fr. »).
            while let Some(c) = url.chars().last().filter(|c| ".,;:!?)]".contains(*c)) {
                url = &url[..url.len() - c.len_utf8()];
            }
            if url.len() > 8 {
                if pos > last {
                    out.push(Run { text: text[last..pos].to_string(), ..run.clone() });
                }
                let href = if url.to_ascii_lowercase().starts_with("www.") { format!("https://{url}") } else { url.to_string() };
                out.push(Run { text: url.to_string(), link: Some(href), underline: true, ..run.clone() });
                pos += url.len();
                last = pos;
                continue;
            }
        }
        pos += text[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    if last < text.len() {
        out.push(Run { text: text[last..].to_string(), ..run });
    }
}

/// Extensions de fichiers considérées comme « à télécharger ».
const FILE_EXT: &[&str] = &[
    "pdf", "zip", "rar", "7z", "tar", "gz", "tgz", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt",
    "ods", "odp", "rtf", "csv", "txt", "png", "jpg", "jpeg", "gif", "webp", "svg", "psd", "ai", "mp4",
    "mov", "avi", "mkv", "mp3", "wav", "ogg", "flac", "fbx", "obj", "blend", "uasset", "umap", "unitypackage",
    "exe", "msi", "dmg", "apk", "iso", "epub", "ics",
];

/// Services de partage / liens de téléchargement connus.
const DOWNLOAD_HINTS: &[&str] = &[
    "drive.google.com/file", "drive.google.com/uc", "drive.google.com/open", "docs.google.com/uc",
    "drive.usercontent.google.com", "export=download", "dl=1", "download=", "/download", "wetransfer.com",
    "we.tl/", "dropbox.com/s/", "dropbox.com/scl/", "dl.dropboxusercontent.com", "1drv.ms/", "onedrive.live.com",
    "sharepoint.com", "swisstransfer.com", "mega.nz", "smash.gg/", "fromsmash.com", "box.com/s/", "attachment",
];

/// Le lien mène-t-il à un fichier à télécharger ?
pub fn is_download(url: &str, text: &str) -> bool {
    let u = url.to_ascii_lowercase();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return false;
    }
    let path = u.split(['?', '#']).next().unwrap_or("");
    let ext_ok = path
        .rsplit('/')
        .next()
        .and_then(|seg| seg.rsplit_once('.'))
        .is_some_and(|(_, ext)| FILE_EXT.contains(&ext));
    let t = text.to_lowercase();
    ext_ok
        || DOWNLOAD_HINTS.iter().any(|h| u.contains(h))
        || t.contains("télécharger")
        || t.contains("telecharger")
        || t.contains("download")
}

const DELIMS: &[&str] = &["**", "__", "~~", "`", "*", "_"];

/// Découpe `text` selon les délimiteurs Markdown, en héritant du style `base`.
pub fn inline_md(text: &str, base: &Run, out: &mut Vec<Run>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut plain_start = 0;
    let mut i = 0;
    while i < chars.len() {
        let pos = chars[i].0;
        let rest = &text[pos..];
        let Some(d) = DELIMS.iter().find(|d| rest.starts_with(**d)) else {
            i += 1;
            continue;
        };
        let after = pos + d.len();
        let prev = text[..pos].chars().next_back();
        let next = text[after..].chars().next();
        // Ouverture : suivie d'un caractère visible ; `_`/`*` simples pas au milieu d'un mot.
        let word_inner = matches!(*d, "_" | "__" | "*") && prev.is_some_and(|c| c.is_alphanumeric());
        let opens = next.is_some_and(|c| !c.is_whitespace()) && !word_inner;
        if opens {
            if let Some(close) = find_close(text, after, d) {
                if pos > plain_start {
                    out.push(Run { text: text[plain_start..pos].to_string(), ..base.clone() });
                }
                let inner = &text[after..close];
                let mut style = base.clone();
                match *d {
                    "**" | "__" => style.bold = true,
                    "~~" => style.strike = true,
                    "`" => style.code = true,
                    _ => style.italic = true,
                }
                if style.code {
                    out.push(Run { text: inner.to_string(), ..style });
                } else {
                    inline_md(inner, &style, out);
                }
                plain_start = close + d.len();
                while i < chars.len() && chars[i].0 < plain_start {
                    i += 1;
                }
                continue;
            }
        }
        // Délimiteur littéral : on saute toute la séquence.
        i += d.chars().count();
    }
    if plain_start < text.len() {
        out.push(Run { text: text[plain_start..].to_string(), ..base.clone() });
    }
}

fn find_close(text: &str, from: usize, d: &str) -> Option<usize> {
    let mut search = from;
    while let Some(rel) = text[search..].find(d) {
        let at = search + rel;
        let prev = text[..at].chars().next_back();
        let next = text[at + d.len()..].chars().next();
        let ok_prev = prev.is_some_and(|c| !c.is_whitespace()) && at > from;
        let word_inner = matches!(d, "_" | "__" | "*") && next.is_some_and(|c| c.is_alphanumeric());
        // `*` simple : ne pas fermer sur le début d'un `**`.
        let doubled = d.len() == 1 && next == d.chars().next();
        if ok_prev && !word_inner && !doubled && !text[from..at].contains('\n') {
            return Some(at);
        }
        search = at + d.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(b: &[BodyBlock]) -> Vec<&Para> {
        b.iter().filter_map(|x| if let BodyBlock::Para(p) = x { Some(p) } else { None }).collect()
    }

    fn run<'a>(p: &'a Para, text: &str) -> &'a Run {
        p.runs.iter().find(|r| r.text.contains(text)).unwrap_or_else(|| panic!("{text:?} absent de {:?}", p.runs))
    }

    #[test]
    fn gmail_html_formatting() {
        // HTML typique produit par l'éditeur de Gmail.
        let html = r#"<div dir="ltr"><div>Bonjour <b>Dorian</b>,</div><div><br></div>
            <div><i>en italique</i> et <u>souligné</u> et <strike>barré</strike></div>
            <div style="text-align:center">centré</div>
            <div><font size="4">grand</font> <span style="font-weight:700">gras css</span></div>
            <ul><li>un</li><li>deux</li></ul>
            <blockquote class="gmail_quote">cité</blockquote>
            <a href="https://ex.com">lien</a></div>"#;
        let b = from_html(html);
        let p = paras(&b);
        assert!(run(p[0], "Dorian").bold);
        assert!(!run(p[0], "Bonjour").bold);
        assert!(p[1].runs.is_empty(), "ligne vide conservée : {:?}", p[1]);
        assert!(run(p[2], "italique").italic);
        assert!(run(p[2], "souligné").underline);
        assert!(run(p[2], "barré").strike);
        assert_eq!(p[3].align, Align::Center);
        assert!(run(p[4], "grand").scale >= 1.3);
        assert!(run(p[4], "gras css").bold);
        assert_eq!(p[5].text(), "•  un");
        assert_eq!(p[6].text(), "•  deux");
        assert_eq!(p[7].quote, 1);
        assert_eq!(run(p[8], "lien").link.as_deref(), Some("https://ex.com"));
    }

    #[test]
    fn images_keep_their_position() {
        let html = r#"<p>Voici :</p><img alt="cap" src="cid:ii_1"><p>Fin</p><img src="https://t.co/p.gif" width="1" height="1">"#;
        let b = from_html(html);
        assert!(matches!(&b[0], BodyBlock::Para(p) if p.text() == "Voici :"));
        assert!(matches!(&b[1], BodyBlock::Image { src: ImageSrc::Cid(c), alt, .. } if c == "ii_1" && alt == "cap"));
        assert!(matches!(&b[2], BodyBlock::Para(p) if p.text() == "Fin"));
        assert_eq!(b.len(), 3, "le pixel de suivi est ignoré");
    }

    #[test]
    fn image_size_and_alignment() {
        let html = r#"<div style="text-align:center"><img src="https://a.fr/1.png" width="300" height="150"></div>
            <div><img src="https://a.fr/2.png" style="width:50%;max-width:600px;display:block;margin:0 auto"></div>
            <table><tr><td align="right"><a href="https://site.fr"><img src="https://a.fr/3.png" width="120"></a></td></tr></table>
            <img src="https://a.fr/4.png" style="float:right" width="80px">"#;
        let imgs: Vec<_> = from_html(html)
            .into_iter()
            .filter_map(|b| if let BodyBlock::Image { width, height, max_width, align, link, .. } = b { Some((width, height, max_width, align, link)) } else { None })
            .collect();
        assert_eq!(imgs[0], (Some(Dim::Px(300.0)), Some(Dim::Px(150.0)), None, Align::Center, None));
        assert_eq!(imgs[1], (Some(Dim::Percent(50.0)), None, Some(Dim::Px(600.0)), Align::Center, None));
        assert_eq!(imgs[2], (Some(Dim::Px(120.0)), None, None, Align::Right, Some("https://site.fr".into())));
        assert_eq!(imgs[3].3, Align::Right);
        assert_eq!(imgs[3].0, Some(Dim::Px(80.0)));
    }

    #[test]
    fn download_links() {
        assert!(is_download("https://ex.com/files/rapport.PDF?x=1", "rapport"));
        assert!(is_download("https://drive.google.com/file/d/abc/view", "le doc"));
        assert!(is_download("https://wetransfer.com/downloads/123", "lien"));
        assert!(is_download("https://ex.com/page", "Télécharger le fichier"));
        assert!(!is_download("https://ex.com/blog/article", "lire l'article"));
        assert!(!is_download("mailto:a@b.fr", "a@b.fr"));

        // Liens HTML + adresses écrites en texte.
        let b = from_html(r#"<p><a href="https://ex.com/a.zip">archive</a> et https://we.tl/t-XYZ. Voir aussi www.site.fr</p>"#);
        let BodyBlock::Para(p) = &b[0] else { panic!() };
        assert!(run(p, "archive").download);
        let we = run(p, "we.tl");
        assert_eq!(we.link.as_deref(), Some("https://we.tl/t-XYZ"));
        assert!(we.download);
        let site = run(p, "www.site.fr");
        assert_eq!(site.link.as_deref(), Some("https://www.site.fr"));
        assert!(!site.download);
        assert!(p.text().contains("t-XYZ. Voir"), "le point final n'est pas dans le lien");
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#888"), Some([0x88, 0x88, 0x88]));
        assert_eq!(parse_color("rgb(255, 0, 10)"), Some([255, 0, 10]));
        assert_eq!(parse_color("Navy"), Some([0, 0, 128]));
        let b = from_html(r##"<p><font color="#e31b23">ARTFX</font> <span style="color:rgb(0,0,255)">bleu</span></p>"##);
        let BodyBlock::Para(p) = &b[0] else { panic!() };
        assert_eq!(run(p, "ARTFX").color, Some([0xe3, 0x1b, 0x23]));
        assert_eq!(run(p, "bleu").color, Some([0, 0, 255]));
    }

    #[test]
    fn markdown_inline() {
        let mut out = Vec::new();
        inline_md("Un **mot** en gras, un *autre* en italique, ~~barré~~ et `code`.", &Run::default(), &mut out);
        let get = |t: &str| out.iter().find(|r| r.text == t).unwrap();
        assert!(get("mot").bold);
        assert!(get("autre").italic);
        assert!(get("barré").strike);
        assert!(get("code").code);
        let all: String = out.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(all, "Un mot en gras, un autre en italique, barré et code.");
    }

    #[test]
    fn markdown_does_not_break_normal_text() {
        for s in ["fichier_de_test.txt", "2 * 3 * 4", "user_name@mail.com", "**pas fermé", "a ** b"] {
            let mut out = Vec::new();
            inline_md(s, &Run::default(), &mut out);
            let all: String = out.iter().map(|r| r.text.as_str()).collect();
            assert_eq!(all, s);
            assert!(out.iter().all(|r| !r.bold && !r.italic), "{s} → {out:?}");
        }
    }

    #[test]
    fn markdown_in_html_and_plain() {
        let b = from_html("<div>C'est **important**</div>");
        assert!(run(paras(&b)[0], "important").bold);

        let b = from_plain("# Titre\nLigne avec **gras**\n\n> citation\n- puce");
        let p = paras(&b);
        assert!(run(p[0], "Titre").bold && run(p[0], "Titre").scale > 1.5);
        assert!(run(p[1], "gras").bold);
        assert_eq!(p[3].quote, 1);
        assert!(p[4].text().starts_with("•  puce"));
    }

    #[test]
    fn plain_text_for_replies() {
        let t = to_text(r#"<a href="https://ex.com">site</a>"#);
        assert!(!t.contains("[1]"), "{t}");
    }
}
