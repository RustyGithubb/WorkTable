//! Correcteur d'orthographe façon Discord : les mots inconnus sont soulignés d'une vague rouge
//! dans les champs de saisie (mail, note, message, tâche, événement…) ; clic droit sur un mot
//! souligné : corrections proposées, « Ajouter au dictionnaire », « Ignorer ».
//!
//! Dictionnaires Hunspell français (Grammalecte, MPL 2.0) et anglais (SCOWL) intégrés à
//! l'exécutable, lus par `spellbook` dans un fil à part au premier usage. Un mot est accepté s'il
//! est connu dans l'une des langues actives. Le mot en cours de frappe n'est souligné qu'une fois
//! terminé. Dictionnaire personnel : `config\dictionary.txt` (un mot par ligne).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use eframe::egui::{self, Color32, Pos2, Stroke, text::CCursor, text_selection::CCursorRange};

use crate::theme;

const FR_AFF: &str = include_str!("../assets/dict/fr.aff");
const FR_DIC: &str = include_str!("../assets/dict/fr.dic");
const EN_AFF: &str = include_str!("../assets/dict/en.aff");
const EN_DIC: &str = include_str!("../assets/dict/en.dic");

/// Couleur de la vague (rouge Discord).
const SQUIGGLE: Color32 = Color32::from_rgb(0xF2, 0x3F, 0x43);
/// Corrections proposées au plus.
const MAX_SUGGESTIONS: usize = 5;

struct Engine {
    fr: Option<spellbook::Dictionary>,
    en: Option<spellbook::Dictionary>,
}

/// Langues actives et activation (Réglages › Orthographe).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Config {
    on: bool,
    fr: bool,
    en: bool,
    /// Langue de l'interface en français : corrections françaises proposées en premier.
    french_first: bool,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
static LOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CONFIG: Mutex<Config> = Mutex::new(Config { on: true, fr: true, en: true, french_first: true });
/// Mot → correct ? (vidé quand les langues ou le dictionnaire personnel changent).
static CACHE: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);
/// Dictionnaire personnel (en minuscules) ; mots ignorés jusqu'à la fermeture de l'app.
static PERSONAL: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static IGNORED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Dictionnaires chargés (le chargement démarre au premier appel, en arrière-plan).
fn engine(ctx: &egui::Context) -> Option<Arc<Engine>> {
    if let Some(e) = ENGINE.get() {
        return Some(e.clone());
    }
    if !LOADING.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("wt-spell".into()).spawn(move || {
            let e = Engine { fr: spellbook::Dictionary::new(FR_AFF, FR_DIC).ok(), en: spellbook::Dictionary::new(EN_AFF, EN_DIC).ok() };
            let _ = ENGINE.set(Arc::new(e));
            ctx.request_repaint();
        });
    }
    None
}

/// Réglages : activation et langues (appelé à chaque image ; le cache est vidé s'ils changent).
pub fn configure(on: bool, fr: bool, en: bool) {
    let french_first = !crate::i18n::en();
    let new = Config { on, fr, en, french_first };
    let mut c = lock(&CONFIG);
    if *c != new {
        *c = new;
        *lock(&CACHE) = None;
    }
}

fn dictionary_path() -> std::path::PathBuf {
    crate::paths::config().join("dictionary.txt")
}

fn personal() -> MutexGuard<'static, Option<HashSet<String>>> {
    let mut p = lock(&PERSONAL);
    if p.is_none() {
        let words = std::fs::read_to_string(dictionary_path()).unwrap_or_default();
        *p = Some(words.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect());
    }
    p
}

/// Mots du dictionnaire personnel, triés.
pub fn personal_words() -> Vec<String> {
    let mut v: Vec<String> = personal().as_ref().map(|s| s.iter().cloned().collect()).unwrap_or_default();
    v.sort();
    v
}

/// Ajoute (ou retire) un mot du dictionnaire personnel, enregistré aussitôt.
pub fn set_personal(word: &str, add: bool) {
    let w = word.trim().to_lowercase();
    if w.is_empty() {
        return;
    }
    let mut p = personal();
    let set = p.get_or_insert_with(HashSet::new);
    if add {
        set.insert(w);
    } else {
        set.remove(&w);
    }
    let mut words: Vec<&String> = set.iter().collect();
    words.sort();
    let text: String = words.iter().map(|w| format!("{w}\n")).collect();
    let _ = std::fs::create_dir_all(crate::paths::config());
    let _ = std::fs::write(dictionary_path(), text);
    drop(p);
    *lock(&CACHE) = None;
}

fn ignore(word: &str) {
    lock(&IGNORED).get_or_insert_with(HashSet::new).insert(word.to_lowercase());
    *lock(&CACHE) = None;
}

// --- Mots ---------------------------------------------------------------------------------

/// Mot du texte : octets et caractères (début, fin).
#[derive(Clone, Debug, PartialEq)]
struct Word {
    bytes: Range<usize>,
    chars: Range<usize>,
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// Mots à vérifier : suites de lettres, avec apostrophes et traits d'union internes. Les morceaux
/// qui ressemblent à une adresse, un lien, une mention, un mot-dièse ou du code sont ignorés,
/// ainsi que les mots avec des chiffres.
fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    // Morceaux séparés par des blancs.
    let mut chunk_start: Option<(usize, usize)> = None;
    let mut ci = 0usize;
    let mut in_code = false;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let flush = |out: &mut Vec<Word>, start: (usize, usize), end_b: usize, in_code: bool| {
        let chunk = &text[start.0..end_b];
        if in_code || chunk.contains("://") || chunk.starts_with("www.") || chunk.contains('@') || chunk.contains('#') || chunk.contains('`') || chunk.contains('\\') {
            return;
        }
        // Mots du morceau.
        let mut cs = chunk.char_indices().peekable();
        let mut c_off = start.1;
        let mut cur: Option<(usize, usize)> = None;
        let mut last_letter = (0usize, 0usize);
        let mut digits = false;
        let push = |out: &mut Vec<Word>, s: (usize, usize), e: (usize, usize), digits: bool| {
            if !digits {
                out.push(Word { bytes: start.0 + s.0..start.0 + e.0, chars: s.1..e.1 });
            }
        };
        while let Some((b, c)) = cs.next() {
            let next_is_letter = cs.peek().is_some_and(|(_, n)| n.is_alphanumeric());
            if c.is_alphanumeric() {
                if cur.is_none() {
                    cur = Some((b, c_off));
                    digits = false;
                }
                digits |= c.is_numeric();
                last_letter = (b + c.len_utf8(), c_off + 1);
            } else if cur.is_some() && (is_apostrophe(c) || c == '-') && next_is_letter {
                // Apostrophe ou trait d'union à l'intérieur du mot.
            } else if let Some(s) = cur.take() {
                push(out, s, last_letter, digits);
            }
            c_off += 1;
        }
        if let Some(s) = cur.take() {
            push(out, s, last_letter, digits);
        }
    };
    for &(b, c) in &chars {
        if c.is_whitespace() {
            if let Some(s) = chunk_start.take() {
                flush(&mut out, s, b, in_code);
            }
        } else {
            if chunk_start.is_none() {
                chunk_start = Some((b, ci));
            }
            if c == '`' {
                // Code en ligne (notes, messages) : rien n'est vérifié jusqu'au ` suivant.
                in_code = text[..b].matches('`').count().is_multiple_of(2);
            }
        }
        ci += 1;
    }
    if let Some(s) = chunk_start.take() {
        flush(&mut out, s, text.len(), in_code);
    }
    out
}

/// Le mot est-il correct (ou à ne pas vérifier) ?
fn word_ok(e: &Engine, cfg: Config, w: &str) -> bool {
    let n = w.chars().count();
    if n < 2 {
        return true;
    }
    // Sigles (« PDF », « RTT ») : pas vérifiés.
    if n <= 6 && w.chars().all(|c| !c.is_lowercase()) {
        return true;
    }
    let lower = w.to_lowercase();
    if personal().as_ref().is_some_and(|p| p.contains(&lower)) || lock(&IGNORED).as_ref().is_some_and(|p| p.contains(&lower)) {
        return true;
    }
    let check = |s: &str| (cfg.fr && e.fr.as_ref().is_some_and(|d| d.check(s))) || (cfg.en && e.en.as_ref().is_some_and(|d| d.check(s)));
    if check(w) {
        return true;
    }
    // Mot composé : chaque partie connue suffit.
    if w.contains('-') && w.split('-').all(|p| p.chars().count() < 2 || check(p)) {
        return true;
    }
    false
}

fn ok_cached(e: &Engine, cfg: Config, w: &str) -> bool {
    if let Some(v) = lock(&CACHE).as_ref().and_then(|c| c.get(w)) {
        return *v;
    }
    let v = word_ok(e, cfg, w);
    lock(&CACHE).get_or_insert_with(HashMap::new).insert(w.to_string(), v);
    v
}

/// Le texte est-il plutôt en français (`Some(true)`) ou en anglais ? D'après ses 60 premiers mots.
fn french_text(e: &Engine, text: &str) -> Option<bool> {
    let (fr, en) = (e.fr.as_ref()?, e.en.as_ref()?);
    let (mut f, mut g) = (0, 0);
    for w in words(text).into_iter().take(60) {
        let w = &text[w.bytes];
        f += usize::from(fr.check(w));
        g += usize::from(en.check(w));
    }
    (f != g).then_some(f > g)
}

/// Corrections proposées (langue de l'interface d'abord), sans les coupures en un mot d'une lettre.
fn suggestions(e: &Engine, cfg: Config, w: &str) -> Vec<String> {
    let mut order: Vec<&spellbook::Dictionary> = Vec::new();
    let (fr, en) = (e.fr.as_ref().filter(|_| cfg.fr), e.en.as_ref().filter(|_| cfg.en));
    if cfg.french_first {
        order.extend(fr);
        order.extend(en);
    } else {
        order.extend(en);
        order.extend(fr);
    }
    let mut out: Vec<String> = Vec::new();
    for d in order {
        let mut s = Vec::new();
        d.suggest(w, &mut s);
        for x in s {
            let split_tiny = x.split([' ', '-']).count() > 1 && x.split([' ', '-']).any(|p| p.chars().count() < 2);
            if !split_tiny && !out.contains(&x) && x != w {
                out.push(x);
            }
        }
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out.truncate(MAX_SUGGESTIONS);
    out
}

// --- Champ de saisie ------------------------------------------------------------------------

/// Menu ouvert sur un mot : champ, mot, position, corrections (calculées en arrière-plan).
#[derive(Clone)]
struct Menu {
    field: egui::Id,
    word: String,
    bytes: Range<usize>,
    at: Pos2,
    suggestions: Arc<Mutex<Option<Vec<String>>>>,
}

fn menu_id() -> egui::Id {
    egui::Id::unique("wt_spell_menu")
}

/// À appeler juste après `TextEdit::show` : souligne les mots inconnus et gère le menu du clic
/// droit. Renvoie `true` si le texte a été corrigé (à traiter comme une modification).
pub fn check(ui: &egui::Ui, out: &egui::text_edit::TextEditOutput, text: &mut String) -> bool {
    let cfg = *lock(&CONFIG);
    // Champ en lecture seule (désactivé) : rien à corriger.
    if !cfg.on || (!cfg.fr && !cfg.en) || !ui.is_enabled() {
        return false;
    }
    let Some(e) = engine(ui.ctx()) else {
        return false;
    };
    let resp = &out.response.response;
    let field = resp.id;
    // Le texte affiché doit correspondre (sinon : indice de texte, mot de passe…).
    if out.galley.text() != text.as_str() || text.is_empty() {
        return menu(ui, field, text, None);
    }
    // Mot en cours de frappe (curseur juste après) : pas encore souligné. Un clic dans un mot ne
    // le fait pas disparaître.
    let typing = resp.has_focus().then_some(out.cursor_range).flatten().map(|r| r.primary.index.0);
    let bad: Vec<Word> = words(text).into_iter().filter(|w| !ok_cached(&e, cfg, &text[w.bytes.clone()])).collect();
    let painter = ui.painter().with_clip_rect(out.text_clip_rect.intersect(ui.clip_rect()));
    for w in bad.iter().filter(|w| !typing.is_some_and(|c| c == w.chars.end)) {
        for (x0, x1, y) in spans(out, w) {
            squiggle(&painter, x0, x1, y);
        }
    }
    // Clic droit sur un mot souligné.
    let mut opened = None;
    if resp.secondary_clicked()
        && let Some(p) = ui.ctx().pointer_interact_pos()
    {
        let c = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
        if let Some(w) = bad.iter().find(|w| c >= w.chars.start && c <= w.chars.end) {
            let word = text[w.bytes.clone()].to_string();
            let cell = Arc::new(Mutex::new(None));
            let (e2, cell2, word2, text2) = (e.clone(), cell.clone(), word.clone(), text.clone());
            let ctx = ui.ctx().clone();
            let _ = std::thread::Builder::new().name("wt-spell-suggest".into()).spawn(move || {
                // Corrections d'abord dans la langue du texte (sinon celle de l'interface).
                let cfg = Config { french_first: french_text(&e2, &text2).unwrap_or(cfg.french_first), ..cfg };
                let s = suggestions(&e2, cfg, &word2);
                *lock(&cell2) = Some(s);
                ctx.request_repaint();
            });
            opened = Some(Menu { field, word, bytes: w.bytes.clone(), at: p, suggestions: cell });
        }
    }
    menu(ui, field, text, opened)
}

/// Segments soulignés d'un mot (un par ligne s'il est coupé) : x début, x fin, y.
fn spans(out: &egui::text_edit::TextEditOutput, w: &Word) -> Vec<(f32, f32, f32)> {
    let mut v: Vec<(f32, f32, f32)> = Vec::new();
    for c in w.chars.clone() {
        let a = out.galley.pos_from_cursor(CCursor::new(c));
        let b = out.galley.pos_from_cursor(CCursor { index: (c + 1).into(), prefer_next_row: false });
        if (b.min.y - a.min.y).abs() > 1.0 {
            continue;
        }
        let y = out.galley_pos.y + a.max.y - 1.0;
        let (x0, x1) = (out.galley_pos.x + a.min.x, out.galley_pos.x + b.min.x);
        match v.last_mut() {
            Some(last) if (last.2 - y).abs() < 1.0 => last.1 = x1,
            _ => v.push((x0, x1, y)),
        }
    }
    v
}

/// Vague rouge sous un mot.
fn squiggle(p: &egui::Painter, x0: f32, x1: f32, y: f32) {
    if x1 - x0 < 2.0 {
        return;
    }
    let (amp, step) = (1.1, 2.0);
    let n = ((x1 - x0) / step).ceil() as usize;
    let pts: Vec<Pos2> = (0..=n).map(|k| Pos2::new((x0 + k as f32 * step).min(x1), y + if k % 2 == 0 { -amp } else { amp })).collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.2, SQUIGGLE)));
}

/// Menu de corrections du champ `field` (ouvert, affiché, choix appliqué).
fn menu(ui: &egui::Ui, field: egui::Id, text: &mut String, opened: Option<Menu>) -> bool {
    let ctx = ui.ctx().clone();
    if let Some(m) = opened {
        ctx.data_mut(|d| d.insert_temp(menu_id(), Some(m)));
    }
    let Some(m) = ctx.data(|d| d.get_temp::<Option<Menu>>(menu_id())).flatten().filter(|m| m.field == field) else {
        return false;
    };
    // Le mot a changé entre-temps : menu fermé.
    if text.get(m.bytes.clone()) != Some(m.word.as_str()) {
        ctx.data_mut(|d| d.insert_temp::<Option<Menu>>(menu_id(), None));
        return false;
    }
    enum Choice {
        Replace(String),
        Add,
        Ignore,
        Close,
    }
    let mut choice = None;
    let area =
        egui::Area::new(egui::Id::unique(("wt_spell_menu_area", field))).order(egui::Order::Foreground).fixed_pos(m.at + egui::vec2(2.0, 4.0)).constrain(true).show(&ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                match lock(&m.suggestions).clone() {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(egui::RichText::new(t!("Recherche de corrections…", "Looking for corrections…")).small().color(theme::TEXT_MUTED()));
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    Some(list) if list.is_empty() => {
                        ui.label(egui::RichText::new(t!("Aucune correction", "No correction")).small().color(theme::TEXT_MUTED()));
                    }
                    Some(list) => {
                        for s in list {
                            if ui.add(egui::Button::new(egui::RichText::new(&s).strong()).frame(false).min_size(egui::vec2(170.0, 0.0))).clicked() {
                                choice = Some(Choice::Replace(s));
                            }
                        }
                    }
                }
                ui.separator();
                if ui.add(egui::Button::new(t!("Ajouter au dictionnaire", "Add to dictionary")).frame(false)).clicked() {
                    choice = Some(Choice::Add);
                }
                if ui.add(egui::Button::new(t!("Ignorer", "Ignore")).frame(false)).clicked() {
                    choice = Some(Choice::Ignore);
                }
            });
        });
    // Clic ailleurs ou Échap : fermé.
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !ctx.pointer_hover_pos().is_some_and(|p| area.response.rect.contains(p));
    if choice.is_none() && (outside || ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
        // Le clic droit qui vient d'ouvrir le menu ne le referme pas.
        if !ctx.input(|i| i.pointer.secondary_pressed()) || !area.response.rect.contains(m.at + egui::vec2(3.0, 5.0)) {
            choice = Some(Choice::Close);
        }
    }
    let Some(choice) = choice else {
        return false;
    };
    ctx.data_mut(|d| d.insert_temp::<Option<Menu>>(menu_id(), None));
    match choice {
        Choice::Replace(s) => {
            let end_chars = text[..m.bytes.start].chars().count() + s.chars().count();
            text.replace_range(m.bytes.clone(), &s);
            // Curseur après le mot corrigé, saisie reprise.
            if let Some(mut st) = egui::text_edit::TextEditState::load(&ctx, field) {
                st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(end_chars))));
                st.store(&ctx, field);
            }
            ctx.memory_mut(|m| m.request_focus(field));
            true
        }
        Choice::Add => {
            set_personal(&m.word, true);
            false
        }
        Choice::Ignore => {
            ignore(&m.word);
            false
        }
        Choice::Close => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<&str> {
        words(text).into_iter().map(|w| &text[w.bytes]).collect()
    }

    #[test]
    fn splits_words_and_skips_links() {
        assert_eq!(found("Bonjour, j'ai vu l’école !"), vec!["Bonjour", "j'ai", "vu", "l’école"]);
        assert_eq!(found("rendez-vous à 14h avec @Paul sur https://x.fr et a@b.fr"), vec!["rendez-vous", "à", "avec", "sur", "et"]);
        assert_eq!(found("le `code faux` ici"), vec!["le", "ici"]);
        assert_eq!(found("« citation »"), vec!["citation"]);
        let w = &words("é bonjoure")[1];
        assert_eq!(w.chars, 2..10);
    }

    #[test]
    fn checks_french_and_english() {
        let e = Engine { fr: spellbook::Dictionary::new(FR_AFF, FR_DIC).ok(), en: spellbook::Dictionary::new(EN_AFF, EN_DIC).ok() };
        let cfg = Config { on: true, fr: true, en: true, french_first: true };
        for w in ["bonjour", "aujourd'hui", "l’école", "étaient", "hello", "receive", "PDF", "porte-monnaie"] {
            assert!(word_ok(&e, cfg, w), "{w}");
        }
        for w in ["bonjoure", "etaient", "recieve"] {
            assert!(!word_ok(&e, cfg, w), "{w}");
        }
        let s = suggestions(&e, Config { french_first: true, ..cfg }, "etaient");
        assert_eq!(s.first().map(String::as_str), Some("étaient"));
        assert!(!s.iter().any(|x| x == "et aient" || x == "bonjour e"));
    }
}
