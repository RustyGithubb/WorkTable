//! Blueprint : automatisations visuelles à la manière d'Unreal Engine 5.
//! À gauche, la liste des graphs (recherche, favoris, création, suppression) ;
//! à droite, l'espace de travail no-code (voir `canvas`).
//!
//! Étape 1 : l'onglet, les graphs et l'espace de travail. Les nodes viendront ensuite.

mod canvas;
mod editor;
mod library;
mod nodes;
pub mod runtime;
pub mod sim;
mod store;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::app::Tab;
use crate::{theme, widgets};
use store::Graph;

pub use editor::icon_tab;
pub use nodes::CATEGORIES;

/// Prochaine occurrence de l'heure « HH:MM » (aussi « 9h30 », « 9 h », « 14 ») : aujourd'hui si
/// elle n'est pas encore passée, sinon demain. Renvoie des millisecondes.
pub fn next_time(text: &str, now: chrono::DateTime<chrono::Local>) -> Option<i64> {
    use chrono::{Duration as Days, TimeZone};
    let t = text.trim().to_lowercase().replace(' ', "");
    let (h, m) = match t.split_once([':', 'h']) {
        Some((h, m)) => (h.parse::<u32>().ok()?, if m.is_empty() { 0 } else { m.parse::<u32>().ok()? }),
        None => (t.parse::<u32>().ok()?, 0),
    };
    let time = chrono::NaiveTime::from_hms_opt(h, m, 0)?;
    let today = now.date_naive().and_time(time);
    let at = chrono::Local.from_local_datetime(&today).single()?;
    let at = if at <= now { at + Days::days(1) } else { at };
    Some(at.timestamp_millis())
}

const LIST_W: f32 = 270.0;
/// Espace entre la liste et le graph, entre le graph et le journal.
const GAP: f32 = 28.0;
const LOG_H: f32 = 150.0;
/// Durée d'une étape de la simulation animée (s), et temps d'arrêt sur la dernière.
const STEP_TIME: f64 = 0.22;
const SIM_HOLD: f64 = 0.8;
/// Durée de l'agrandissement en plein écran et du retour (s).
const FULL_TIME: f32 = 0.35;

#[derive(Default)]
pub struct BlueprintState {
    loaded: bool,
    graphs: Vec<Graph>,
    selected: Option<String>,
    query: String,
    /// Renommage dans la liste : graph, texte, focus à donner.
    rename: Option<(String, String, bool)>,
    confirm_delete: Option<String>,
    canvas: canvas::Canvas,
    /// Modifications à enregistrer : graph → (dernière modification, date de modif. à changer).
    dirty: Vec<(String, Instant, bool)>,
    /// Simulation : résultat, début (temps de l'interface) et heure de lancement.
    trace: Option<sim::Trace>,
    sim_started: Option<f64>,
    sim_at: Option<chrono::DateTime<chrono::Local>>,
    log_open: bool,
    /// Fenêtre des données d'exemple ouverte, et sa position (sous la pilule de commandes).
    sample_open: bool,
    sample_pos: Pos2,
    /// Le graph occupe toute la fenêtre.
    fullscreen: bool,
    /// Exécution réelle des graphs actifs.
    engine: runtime::Engine,
    /// Journal : simulation (faux) ou exécutions réelles (vrai).
    log_runs: bool,
    /// Annuler / rétablir, par graph.
    history: std::collections::HashMap<String, History>,
    /// Nodes copiées (aussi mises dans le presse-papiers du système, pour un autre graph).
    clipboard: Option<store::Clip>,
}

/// Historique d'un graph : état de référence (dernier état stable), états précédents et
/// états annulés.
struct History {
    base: store::Snap,
    undo: Vec<store::Snap>,
    redo: Vec<store::Snap>,
}

/// États gardés pour « Annuler ».
const UNDO_MAX: usize = 100;

impl Drop for BlueprintState {
    fn drop(&mut self) {
        self.flush(true);
        if self.loaded {
            self.engine.save(true);
        }
    }
}

impl BlueprintState {
    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.loaded = true;
            self.graphs = Graph::load_all();
            self.graphs.sort_by_key(|g| -g.updated);
            self.selected = self.graphs.first().map(|g| g.id.clone());
            self.engine.load();
        }
    }

    /// Variables changées par les exécutions réelles : à enregistrer.
    fn engine_touched(&mut self) {
        for id in std::mem::take(&mut self.engine.touched) {
            self.mark(&id, false);
        }
    }

    fn mark(&mut self, id: &str, touch: bool) {
        match self.dirty.iter_mut().find(|d| d.0 == id) {
            Some(d) => {
                d.1 = Instant::now();
                d.2 |= touch;
            }
            None => self.dirty.push((id.to_string(), Instant::now(), touch)),
        }
    }

    /// Enregistre les graphs modifiés depuis plus d'une demi-seconde (ou tous si `all`).
    fn flush(&mut self, all: bool) {
        let graphs = &mut self.graphs;
        self.dirty.retain(|(id, at, touch)| {
            if !all && at.elapsed() < Duration::from_millis(500) {
                return true;
            }
            if let Some(g) = graphs.iter_mut().find(|g| &g.id == id) {
                g.save(*touch);
            }
            false
        });
    }

    fn selected_mut(&mut self) -> Option<&mut Graph> {
        let id = self.selected.clone()?;
        self.graphs.iter_mut().find(|g| g.id == id)
    }

    fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_string());
            self.canvas = canvas::Canvas::default();
            self.sim_started = None;
            self.trace = None;
        }
    }

    fn create(&mut self) {
        self.ensure_loaded();
        let name = store::unique_name(t!("Nouveau graph", "New graph"), &self.graphs);
        let mut g = Graph::new(&name);
        g.save(false);
        let id = g.id.clone();
        self.graphs.insert(0, g);
        self.select(&id);
        self.rename = Some((id, name, true));
    }

    /// Active ou désactive l'exécution réelle d'un graph.
    fn toggle_active(&mut self, id: &str) {
        if let Some(g) = self.graphs.iter_mut().find(|g| g.id == id) {
            g.active = !g.active;
        }
        self.mark(id, false);
    }

    fn duplicate(&mut self, id: &str) {
        let Some(src) = self.graphs.iter().find(|g| g.id == id) else { return };
        let name = store::unique_name(&tf!("{} (copie)", "{} (copy)", src.name), &self.graphs);
        let mut g = src.duplicate(&name);
        g.save(false);
        let new_id = g.id.clone();
        self.graphs.insert(0, g);
        self.select(&new_id);
    }

    /// Enregistre une copie du graph dans un fichier choisi (pour le partager).
    fn export(&mut self, id: &str) {
        let Some(g) = self.graphs.iter().find(|g| g.id == id) else { return };
        let name: String = g.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let Some(path) = rfd::FileDialog::new().add_filter("Blueprint", &["json"]).set_file_name(format!("{}.json", name.trim())).save_file() else { return };
        // Sans l'état propre à cet ordinateur : actif, favori, variables.
        let shared = Graph { active: false, favorite: false, memory: Default::default(), ..g.clone() };
        crate::paths::save_json(&path, &shared);
    }

    /// Ajoute les graphs des fichiers `paths` (exportés par WorkTable) ; ils arrivent inactifs.
    fn import(&mut self, paths: &[std::path::PathBuf]) {
        self.ensure_loaded();
        let mut last = None;
        for p in paths {
            let Some(src) = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Graph>(&t).ok()) else { continue };
            let name = store::unique_name(src.name.trim(), &self.graphs);
            let mut g = src.duplicate(&name);
            g.memory.clear();
            g.save(false);
            last = Some(g.id.clone());
            self.graphs.insert(0, g);
        }
        if let Some(id) = last {
            self.select(&id);
        }
    }

    fn delete(&mut self, id: &str) {
        if let Some(k) = self.graphs.iter().position(|g| g.id == id) {
            self.graphs.remove(k).delete();
            self.dirty.retain(|d| d.0 != id);
            if self.selected.as_deref() == Some(id) {
                self.selected = None;
                if let Some(first) = self.graphs.first().map(|g| g.id.clone()) {
                    self.select(&first);
                }
            }
        }
    }

    // --- Exécution réelle (appelée par l'app à chaque arrivée) ---------------------------

    /// Mails reçus (`initial` : chargement complet, seuls les mails arrivés app fermée sont
    /// rattrapés). Le contenu complet des mails à exécuter est demandé à Gmail ; les graphs
    /// tournent à sa réception (`on_full_mail`).
    pub fn on_mails(&mut self, list: &[crate::google::gmail::Summary], initial: bool, acc: &crate::google::Account) {
        self.ensure_loaded();
        for m in self.engine.on_mails(&self.graphs, list, initial) {
            acc.run(move |s| {
                // Contenu introuvable : on garde au moins l'extrait.
                let full =
                    crate::google::gmail::get_full(s, &m.id).unwrap_or_else(|_| crate::google::gmail::FullMessage { body: m.snippet.clone(), summary: m, ..Default::default() });
                Ok(crate::google::Event::BlueprintMail(Box::new(full)))
            });
        }
    }

    /// Contenu complet d'un mail reçu : exécution des graphs.
    pub fn on_full_mail(&mut self, full: &crate::google::gmail::FullMessage, labels: &[crate::google::gmail::Label], dnd: bool) {
        self.ensure_loaded();
        self.engine.on_full_mail(&mut self.graphs, full, labels, dnd);
        self.engine_touched();
    }

    pub fn on_chat_spaces(&mut self, chat: &crate::tabs::messages::ChatState, acc: &crate::google::Account) {
        self.ensure_loaded();
        self.engine.on_chat_spaces(&self.graphs, chat, acc);
    }

    pub fn on_chat_messages(&mut self, space: &str, list: &[crate::google::chat::ChatMessage], chat: &crate::tabs::messages::ChatState, dnd: bool) {
        self.ensure_loaded();
        self.engine.on_chat_messages(&mut self.graphs, space, list, chat, dnd);
        self.engine_touched();
    }

    /// Rappel minuté qui sonne.
    pub fn on_timer(&mut self, t: &crate::tasks::Timer, late: bool, dnd: bool) {
        self.ensure_loaded();
        self.engine.on_timer(&mut self.graphs, t, late, dnd);
        self.engine_touched();
    }

    /// Rendez-vous qui commencent ; renvoie les effets à appliquer (toutes arrivées confondues).
    pub fn background(&mut self, cal: &crate::tabs::calendar::CalendarState, prefs: &crate::prefs::Prefs) -> Vec<runtime::Applied> {
        self.ensure_loaded();
        self.engine.background(&mut self.graphs, cal, prefs);
        self.engine_touched();
        let fx = self.engine.take();
        if !fx.is_empty() {
            self.log_runs = true;
        }
        fx
    }

    /// Lance la simulation du graph ouvert, de l'event jusqu'au bout du flux.
    fn simulate(&mut self, now: f64) {
        let Some(g) = self.selected_mut() else { return };
        let mut trace = sim::run(g);
        let name = g.name.clone();
        trace.lines.insert(0, sim::Line { step: 0, warn: false, text: tf!("Simulation de « {name} » (données d'exemple)", "Simulating “{name}” (sample data)") });
        self.trace = Some(trace);
        self.sim_started = Some(now);
        self.sim_at = Some(chrono::Local::now());
        self.log_open = true;
        self.log_runs = false;
    }

    /// Avancement de la simulation animée : (étapes jouées, avancement de la dernière, en cours ?).
    fn sim_progress(&self, now: f64) -> Option<(usize, f32, bool)> {
        let (trace, start) = (self.trace.as_ref()?, self.sim_started?);
        let n = trace.steps.len();
        let t = now - start;
        let played = ((t / STEP_TIME) as usize + 1).min(n);
        let frac = ((t / STEP_TIME).fract()) as f32;
        let running = t < n as f64 * STEP_TIME + SIM_HOLD;
        Some((played, if played < n { frac } else { 1.0 }, running))
    }
}

/// Pose la node choisie dans la bibliothèque ; si un fil avait été tiré, la node est placée
/// pour que la pin reliée tombe sous le pointeur.
fn place_node(g: &mut Graph, lib: &library::Library, kind: &'static str) -> u64 {
    let at = [lib.graph.x, lib.graph.y];
    let id = g.add_node(kind, at);
    let Some(def) = nodes::get(kind) else { return id };
    if let Some(from) = &lib.from
        && let Some(key) = library::compatible_pin(def, from)
    {
        let n = g.node(id).expect("node posée");
        let l = editor::layout(n, def);
        let pin = if from.output { l.inputs.iter().find(|p| p.0.key == key) } else { l.outputs.iter().find(|p| p.0.key == key) }.map(|p| p.1);
        if let (Some(pin), Some(n)) = (pin, g.node_mut(id)) {
            n.pos = [n.pos[0] + lib.graph.x - pin.x, n.pos[1] + lib.graph.y - pin.y];
        }
        let to = editor::PinRef { node: id, key, output: !from.output, ty: if from.output { def.input(key) } else { def.output(key) }.map(|p| p.ty).unwrap_or(from.ty) };
        editor::connect(g, from, &to);
    }
    // Aimantée à la grille, comme quand on la déplace.
    if let Some(n) = g.node_mut(id) {
        n.pos = [editor::snap(n.pos[0]), editor::snap(n.pos[1])];
    }
    id
}

pub fn show(ui: &mut egui::Ui, st: &mut BlueprintState, prefs: &mut crate::prefs::Prefs) {
    st.ensure_loaded();
    st.flush(false);
    if !st.dirty.is_empty() {
        ui.ctx().request_repaint_after(Duration::from_millis(550));
    }
    // Plein écran : sans graph ouvert, il n'y a rien à agrandir.
    if st.selected.is_none() {
        st.fullscreen = false;
    }
    // Échap quitte le plein écran (sauf si la bibliothèque ou un renommage la prend).
    if st.fullscreen && st.canvas.library.is_none() && st.rename.is_none() && !st.sample_open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.fullscreen = false;
    }

    let mut toggle_side = false;
    // Plein écran : le graph s'étend de sa place jusqu'à toute la fenêtre (ease in-out) ; le titre
    // et la liste des graphs restent, en flottant par-dessus.
    let window = ui.ctx().content_rect();
    let kf = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("wt_bp_full"), st.fullscreen, FULL_TIME, egui::emath::easing::cubic_in_out);

    // Le titre, la liste et le graph ont chacun leur calque, en permanence : un calque créé au
    // moment du plein écran resterait invisible une image (egui le mesure d'abord), d'où un
    // clignotement. Le contenu de l'onglet ne fait que réserver leur place.
    let n = st.graphs.len();
    let header_pos = ui.cursor().min;
    let header_w = ui.available_width();
    ui.scope(|ui| {
        ui.set_invisible();
        header(ui, n);
    });
    // Le bloc invisible compte comme un élément : on retire l'espacement ajouté après lui.
    ui.add_space(-ui.spacing().item_spacing.y);
    // Calque du titre sur toute la largeur : sans interaction, pour ne pas cacher l'heure et
    // « Ne pas déranger » en dessous (egui donnerait les clics au calque du dessus).
    float(ui, "wt_bp_header_float", egui::Order::Foreground, Rect::from_min_size(header_pos, vec2(header_w, 0.0)), false, |ui| header(ui, n));
    let full = ui.available_rect_before_wrap();
    // Liste des graphs masquable (rebond élastique, comme les autres onglets).
    let k = widgets::reveal(ui.ctx(), "wt_bp_side", !prefs.blueprint_side_hidden);
    let side_w = widgets::reveal_width(LIST_W, k);
    let gap = if side_w > 0.0 { GAP * k.min(1.0) } else { 0.0 };
    let main_rect = Rect::from_min_max(Pos2::new(full.left() + side_w + gap, full.top()), full.max);
    let side_hidden = prefs.blueprint_side_hidden;

    let grid = prefs.blueprint.grid_opacity;
    workspace(ui, st, main_rect, window, kf, side_hidden, grid, &mut toggle_side);
    if side_w > 0.0 {
        let list_rect = Rect::from_min_size(full.min, vec2(side_w, full.height()));
        float(ui, "wt_bp_list_float", egui::Order::Foreground, list_rect, false, |ui| {
            widgets::animated_column(ui, side_w, LIST_W, k, |ui| {
                widgets::pane(ui, |ui| list(ui, st));
            });
        });
    }
    ui.allocate_rect(full, Sense::hover());
    if toggle_side {
        prefs.blueprint_side_hidden = !prefs.blueprint_side_hidden;
    }

    if let Some(lib) = &mut st.canvas.library {
        let out = library::show(ui.ctx(), lib);
        if let Some(kind) = out.pick
            && let Some(lib) = st.canvas.library.take()
            && let Some(id) = st.selected.clone()
            && let Some(g) = st.graphs.iter_mut().find(|g| g.id == id)
        {
            let node = place_node(g, &lib, kind);
            st.canvas.ed.selected = [node].into_iter().collect();
            st.mark(&id, true);
        } else if !out.keep {
            st.canvas.library = None;
        }
    }
    sample_window(ui.ctx(), st);
    delete_dialog(ui.ctx(), st);
    // Fichiers .json déposés sur l'onglet : importés.
    let dropped: Vec<std::path::PathBuf> =
        ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))).collect());
    if !dropped.is_empty() {
        st.import(&dropped);
    }
}

// --- Liste des graphs ----------------------------------------------------------------

fn list(ui: &mut egui::Ui, st: &mut BlueprintState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let new = new_graph_button(ui, Some(ui.available_width() - 40.0));
        if new.clicked() {
            st.create();
        }
        let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!(
            "Importer un graph (fichier .json exporté par WorkTable). Tu peux aussi glisser le fichier sur l'onglet.",
            "Import a graph (.json file exported by WorkTable). You can also drop the file on the tab."
        ));
        let fill = if resp.hovered() { theme::gray(0x2E) } else { ui.visuals().widgets.inactive.weak_bg_fill };
        ui.painter().rect_filled(r, CornerRadius::same(10), fill);
        draw_transfer(ui.painter(), r.center(), theme::TEXT(), true);
        if resp.clicked()
            && let Some(paths) = rfd::FileDialog::new().add_filter("Blueprint", &["json"]).pick_files()
        {
            st.import(&paths);
        }
    });
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(&mut st.query).hint_text(t!("🔍  Rechercher", "🔍  Search")).desired_width(f32::INFINITY).margin(vec2(10.0, 6.0)));
    ui.add_space(10.0);

    let q = st.query.trim().to_lowercase();
    let matches = |g: &Graph| q.is_empty() || g.name.to_lowercase().contains(&q);
    let favs: Vec<usize> = (0..st.graphs.len()).filter(|&i| st.graphs[i].favorite && matches(&st.graphs[i])).collect();
    let all: Vec<usize> = (0..st.graphs.len()).filter(|&i| matches(&st.graphs[i])).collect();

    let mut act: Option<(String, Act)> = None;
    egui::ScrollArea::vertical().id_salt("wt_bp_list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if !favs.is_empty() {
            section(ui, t!("FAVORIS", "FAVOURITES"), favs.len());
            for &i in &favs {
                row(ui, st, i, "fav", &mut act);
            }
            ui.add_space(14.0);
        }
        section(ui, "GRAPHS", all.len());
        if all.is_empty() {
            widgets::empty_state(
                ui,
                if q.is_empty() { t!("Aucun graph", "No graphs") } else { t!("Aucun résultat", "No results") },
                if q.is_empty() { t!("Crée ton premier graph.", "Create your first graph.") } else { "" },
            );
        }
        for &i in &all {
            row(ui, st, i, "all", &mut act);
        }
    });
    if let Some((id, a)) = act {
        match a {
            Act::Open => st.select(&id),
            Act::Favorite => {
                if let Some(g) = st.graphs.iter_mut().find(|g| g.id == id) {
                    g.favorite = !g.favorite;
                }
                st.mark(&id, false);
            }
            Act::Rename => {
                let name = st.graphs.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default();
                st.rename = Some((id, name, true));
            }
            Act::Duplicate => st.duplicate(&id),
            Act::Export => st.export(&id),
            Act::Active => st.toggle_active(&id),
            Act::Delete => st.confirm_delete = Some(id),
        }
    }
}

#[derive(Clone, Copy)]
enum Act {
    Open,
    Favorite,
    Rename,
    Duplicate,
    Export,
    Active,
    Delete,
}

/// Bouton « + Nouveau graph » (commun aux onglets, voir `widgets::new_button`).
fn new_graph_button(ui: &mut egui::Ui, width: Option<f32>) -> egui::Response {
    widgets::new_button(ui, t!("Nouveau graph", "New graph"), width)
}

fn section(ui: &mut egui::Ui, title: &str, n: usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).small().strong().color(theme::TEXT_MUTED()));
        ui.label(RichText::new(n.to_string()).small().color(theme::TEXT_FAINT()));
    });
    ui.add_space(4.0);
}

fn row(ui: &mut egui::Ui, st: &mut BlueprintState, i: usize, salt: &str, act: &mut Option<(String, Act)>) {
    let g = &st.graphs[i];
    let selected = st.selected.as_deref() == Some(&g.id);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let id = egui::Id::unique(("wt_bp_row", salt, &g.id));

    // Renommage sur place (une seule des deux lignes si le graph est aussi en favori).
    if salt == "all"
        && let Some((rid, text, focus)) = &mut st.rename
        && *rid == g.id
    {
        let edit = ui.put(r.shrink2(vec2(4.0, 2.0)), egui::TextEdit::singleline(text).margin(vec2(8.0, 5.0)));
        if *focus {
            edit.request_focus();
            *focus = false;
        }
        let done = edit.lost_focus();
        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if done || cancel {
            let (rid, text, _) = st.rename.take().unwrap_or_default();
            let name = text.trim();
            if !cancel && !name.is_empty() {
                if let Some(g) = st.graphs.iter_mut().find(|g| g.id == rid) {
                    g.name = name.to_string();
                }
                st.mark(&rid, true);
            }
        }
        return;
    }

    let resp = ui.interact(r, id, Sense::click());
    let over = ui.rect_contains_pointer(r);
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(8), theme::SURFACE_HI());
    } else if over {
        ui.painter().rect_filled(r, CornerRadius::same(8), theme::gray(0x1B));
    }
    let p = ui.painter();
    let col = if selected { theme::TEXT() } else { theme::TEXT_MUTED() };
    crate::sidebar::draw_icon(p, Rect::from_center_size(Pos2::new(r.left() + 16.0, r.center().y), vec2(15.0, 15.0)), Tab::Blueprint, col, 1.0);
    let star_r = Rect::from_center_size(Pos2::new(r.right() - 16.0, r.center().y), vec2(22.0, 22.0));
    let dg = p.layout_no_wrap(crate::tabs::notes::ago(g.updated), egui::FontId::proportional(11.0), theme::TEXT_FAINT());
    let mut right = if over || g.favorite { star_r.left() - 4.0 } else { r.right() - 10.0 - dg.size().x - 6.0 };
    // Graph actif : point vert avant la date (ou l'étoile).
    if g.active {
        p.circle_filled(Pos2::new(right - 4.0, r.center().y), 3.5, ACTIVE);
        right -= 14.0;
    }
    let title = crate::tabs::mail::view::one_line(p, &g.name, egui::FontId::proportional(13.5), if selected { theme::TEXT() } else { theme::gray(0xCC) }, right - r.left() - 34.0);
    p.galley(Pos2::new(r.left() + 32.0, r.center().y - title.size().y / 2.0), title, theme::TEXT());
    if over || g.favorite {
        let s = ui.interact(star_r, id.with("star"), Sense::click()).on_hover_text(if g.favorite {
            t!("Retirer des favoris", "Remove from favourites")
        } else {
            t!("Ajouter aux favoris", "Add to favourites")
        });
        let c = if g.favorite {
            theme::TODAY()
        } else if s.hovered() {
            theme::TEXT()
        } else {
            theme::TEXT_FAINT()
        };
        crate::tabs::mail::view::draw_star(ui.painter(), star_r.center(), 6.5, g.favorite, c);
        if s.clicked() {
            *act = Some((g.id.clone(), Act::Favorite));
        }
    } else {
        ui.painter().galley(Pos2::new(r.right() - 10.0 - dg.size().x, r.center().y - dg.size().y / 2.0), dg, theme::TEXT_FAINT());
    }
    if resp.clicked() {
        *act = Some((g.id.clone(), Act::Open));
    }
    if resp.double_clicked() {
        *act = Some((g.id.clone(), Act::Rename));
    }
    resp.context_menu(|ui| {
        if menu_item(ui, t!("Renommer", "Rename"), None, draw_pencil).clicked() {
            *act = Some((g.id.clone(), Act::Rename));
        }
        if menu_item(ui, t!("Dupliquer", "Duplicate"), None, draw_copy).clicked() {
            *act = Some((g.id.clone(), Act::Duplicate));
        }
        if menu_item(ui, t!("Exporter…", "Export…"), None, |p, c, col| draw_transfer(p, c, col, false)).clicked() {
            *act = Some((g.id.clone(), Act::Export));
        }
        let label = if g.active { t!("Désactiver", "Deactivate") } else { t!("Activer", "Activate") };
        if menu_item(ui, label, None, |p, c, col| draw_switch(p, c, 0.75, g.active, col)).clicked() {
            *act = Some((g.id.clone(), Act::Active));
        }
        let (label, filled) = if g.favorite { (t!("Retirer des favoris", "Remove from favourites"), false) } else { (t!("Ajouter aux favoris", "Add to favourites"), true) };
        if menu_item(ui, label, None, |p, c, col| crate::tabs::mail::view::draw_star(p, c, 6.0, filled, col)).clicked() {
            *act = Some((g.id.clone(), Act::Favorite));
        }
        ui.separator();
        if menu_item(ui, t!("Supprimer", "Delete"), Some(theme::TODAY()), draw_trash).clicked() {
            *act = Some((g.id.clone(), Act::Delete));
        }
    });
}

/// Ligne de menu avec une icône dessinée à gauche (les caractères ★ ⧉ ✏ 🗑 manquent dans
/// certaines polices et s'afficheraient en carré vide).
fn menu_item(ui: &mut egui::Ui, label: &str, color: Option<Color32>, icon: impl FnOnce(&egui::Painter, Pos2, Color32)) -> egui::Response {
    let mut text = RichText::new(format!("       {label}"));
    if let Some(c) = color {
        text = text.color(c);
    }
    let resp = ui.button(text);
    let ink = color.unwrap_or_else(|| ui.style().interact(&resp).text_color());
    let c = Pos2::new(resp.rect.left() + ui.spacing().button_padding.x + 7.0, resp.rect.center().y);
    icon(ui.painter(), c, ink);
    resp
}

fn draw_pencil(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    let (a, b) = (c + vec2(-4.5, 4.5), c + vec2(4.0, -4.0));
    p.line_segment([a + vec2(1.6, 0.0), b + vec2(1.2, 1.2)], s);
    p.line_segment([a + vec2(0.0, -1.6), b - vec2(1.2, 1.2)], s);
    p.line_segment([b - vec2(1.2, 1.2), b + vec2(1.2, 1.2)], s);
    p.line_segment([a + vec2(0.0, -1.6), a + vec2(1.6, 0.0)], s);
}

fn draw_copy(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.4, color);
    let k = CornerRadius::same(2);
    p.rect_stroke(Rect::from_min_size(c + vec2(-5.0, -5.0), vec2(7.0, 7.0)), k, s, egui::StrokeKind::Inside);
    p.rect_stroke(Rect::from_min_size(c + vec2(-2.0, -2.0), vec2(7.0, 7.0)), k, s, egui::StrokeKind::Inside);
}

/// Bac et flèche : vers le bas pour importer, vers le haut pour exporter.
fn draw_transfer(p: &egui::Painter, c: Pos2, color: Color32, down: bool) {
    let s = Stroke::new(1.5, color);
    p.add(egui::Shape::line(vec![c + vec2(-6.0, 1.5), c + vec2(-6.0, 5.5), c + vec2(6.0, 5.5), c + vec2(6.0, 1.5)], s));
    p.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 2.5)], s);
    let (tip, wing) = if down { (2.5, -1.0) } else { (-6.0, 1.0) };
    p.add(egui::Shape::line(vec![c + vec2(-3.0, tip + wing * 3.0), c + vec2(0.0, tip), c + vec2(3.0, tip + wing * 3.0)], s));
}

fn draw_trash(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.4, color);
    p.hline(c.x - 5.0..=c.x + 5.0, c.y - 3.5, s);
    p.hline(c.x - 1.5..=c.x + 1.5, c.y - 5.5, s);
    p.rect_stroke(Rect::from_min_max(c + vec2(-3.5, -3.5), c + vec2(3.5, 5.5)), CornerRadius::same(1), s, egui::StrokeKind::Inside);
}

// --- Espace de travail ---------------------------------------------------------------

/// Commande choisie dans la pilule du graph.
enum Control {
    ToggleSide,
    ZoomOut,
    ZoomReset,
    ZoomIn,
    Recenter,
    Fullscreen,
    Sample,
    Active,
    Simulate,
}

fn header(ui: &mut egui::Ui, n: usize) {
    widgets::page_header_with(ui, t!("Automatisations", "Automations"), "Blueprint", |ui| {
        let plural = if crate::i18n::en() { n != 1 } else { n > 1 };
        widgets::header_badge(ui, &format!("{n} graph{}", if plural { "s" } else { "" }), false);
    });
}

/// Contenu dessiné dans son propre calque, à l'emplacement `rect` (hauteur 0 : selon le contenu).
/// Contenu dessiné dans son propre calque, à l'emplacement `rect` (hauteur 0 : selon le contenu).
/// `fade` : fondu d'apparition d'egui (sinon le calque apparaît tel quel à l'ouverture de l'onglet).
///
/// La toute première fois, egui mesure un nouveau calque sans l'afficher (une image vide à la
/// première ouverture de l'onglet) : cette fois-là, le contenu est dessiné sur place pendant que
/// le calque se mesure, invisible.
fn float(ui: &mut egui::Ui, salt: &str, order: egui::Order, rect: Rect, fade: bool, body: impl FnOnce(&mut egui::Ui)) {
    let id = egui::Id::unique(salt);
    let interactable = salt != "wt_bp_header_float";
    let area = egui::Area::new(id).order(order).fixed_pos(rect.min).constrain(false).movable(false).interactable(interactable).fade_in(fade);
    let place = |ui: &mut egui::Ui, body: Box<dyn FnOnce(&mut egui::Ui) + '_>| {
        if rect.height() > 0.0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), body);
        } else {
            ui.set_width(rect.width());
            body(ui);
        }
    };
    if egui::AreaState::load(ui.ctx(), id).is_none() {
        area.show(ui.ctx(), |ui| ui.allocate_space(rect.size()));
        let mut here = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.min, vec2(rect.width(), rect.height().max(1.0)))));
        place(&mut here, Box::new(body));
        ui.ctx().request_repaint();
    } else {
        area.show(ui.ctx(), |ui| place(ui, Box::new(body)));
    }
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

/// Graph et ses commandes. `rect` : place normale ; `window` : toute la fenêtre ;
/// `kf` : avancement du plein écran (0 = normal, 1 = plein écran).
/// `grid` : opacité de la grille (Réglages › Blueprint).
#[allow(clippy::too_many_arguments)]
fn workspace(ui: &mut egui::Ui, st: &mut BlueprintState, rect: Rect, window: Rect, kf: f32, side_hidden: bool, grid: f32, toggle_side: &mut bool) {
    let now = ui.input(|i| i.time);
    let progress = st.sim_progress(now);
    let simulating = progress.is_some_and(|p| p.2);
    if simulating {
        ui.ctx().request_repaint();
    }
    let Some(idx) = st.selected.as_ref().and_then(|id| st.graphs.iter().position(|g| &g.id == id)) else {
        ui.painter().rect(rect, CornerRadius::same(10), canvas::BG, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
            ui.add_space(rect.height() / 2.0 - 40.0);
            ui.label(RichText::new(t!("Aucun graph ouvert", "No graph open")).size(16.0).color(Color32::from_gray(0xA0)));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                if side_hidden && ui.button(t!("»  Afficher la liste", "»  Show the list")).clicked() {
                    *toggle_side = true;
                }
                if new_graph_button(ui, Some(180.0)).clicked() {
                    st.create();
                }
            });
        });
        return;
    };

    // Place normale du graph (le journal en dessous), place en plein écran (toute la fenêtre) ;
    // le contenu du graph reste centré sur la place normale : il ne bouge pas pendant l'animation.
    let log_h = if st.log_open { LOG_H + GAP } else { 0.0 };
    let normal = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.bottom() - log_h));
    let canvas_rect = lerp_rect(normal, window, kf);
    let mut changes = canvas::Changes::default();
    let (g, cv) = (&mut st.graphs[idx], &mut st.canvas);
    let sim_view = match (&st.trace, progress) {
        (Some(trace), Some((shown, frac, true))) => Some(editor::SimView { trace, shown, frac }),
        _ => None,
    };
    // Calque d'arrière-plan à part, au-dessus du contenu de l'onglet ; l'heure, le titre, la
    // liste et les barres de l'app restent au-dessus.
    float(ui, "wt_bp_canvas_float", egui::Order::Background, canvas_rect, st.fullscreen, |ui| {
        // Ancré au centre de la place du graph journal compris : ouvrir le journal ne le décale pas.
        changes = canvas::show(ui, canvas_rect, normal, rect.center(), g, cv, sim_view.as_ref(), kf, grid);
        if kf > 0.0 {
            inner_shadow(ui.painter(), canvas_rect, kf);
        }
        ui.allocate_rect(canvas_rect, Sense::hover());
    });
    // Pilule de commandes en haut à gauche de la place normale du graph.
    let zoom = st.graphs[idx].view.zoom;
    st.sample_pos = normal.min + vec2(12.0, 56.0);
    let active = st.graphs[idx].active;
    let control = controls(ui.ctx(), normal.min + vec2(12.0, 12.0), st.fullscreen, side_hidden, zoom, simulating, active);
    if st.log_open {
        // Sous le graph ; en plein écran, il flotte en bas de la fenêtre.
        let below = Rect::from_min_max(Pos2::new(rect.left(), normal.bottom() + GAP), rect.max);
        let floating = Rect::from_min_max(Pos2::new(rect.left(), window.bottom() - 16.0 - LOG_H), Pos2::new(window.right() - 16.0, window.bottom() - 16.0));
        log_panel(ui, lerp_rect(below, floating, kf), st, now);
    }

    let g = &mut st.graphs[idx];
    match control {
        Some(Control::ToggleSide) => *toggle_side = true,
        Some(Control::ZoomOut) => canvas::zoom_step(g, 0.8),
        Some(Control::ZoomIn) => canvas::zoom_step(g, 1.25),
        Some(Control::ZoomReset) => g.view.zoom = 1.0,
        Some(Control::Recenter) => canvas::recenter(g),
        Some(Control::Fullscreen) => {
            st.fullscreen = !st.fullscreen;
            st.canvas.library = None;
        }
        Some(Control::Simulate) => {
            st.canvas.library = None;
            st.simulate(now);
        }
        Some(Control::Sample) => st.sample_open = !st.sample_open,
        Some(Control::Active) => {
            let id = st.graphs[idx].id.clone();
            st.toggle_active(&id);
        }
        None => {}
    }
    let view_changed = changes.view || matches!(control, Some(Control::ZoomOut | Control::ZoomIn | Control::ZoomReset | Control::Recenter));
    let id = st.graphs[idx].id.clone();
    if edit_keys(ui.ctx(), st, idx, canvas_rect, rect.center()) {
        changes.graph = true;
    }
    if changes.graph {
        st.mark(&id, true);
    } else if view_changed {
        st.mark(&id, false);
    }
}

/// Annuler / rétablir (Ctrl + Z, Ctrl + Y ou Ctrl + Maj + Z), copier / couper / coller
/// (Ctrl + C, X, V). Renvoie `true` si le graph a changé.
fn edit_keys(ctx: &egui::Context, st: &mut BlueprintState, idx: usize, canvas: Rect, center: Pos2) -> bool {
    let g = &st.graphs[idx];
    let gid = g.id.clone();
    let now = store::Snap::of(g);
    let h = st.history.entry(gid.clone()).or_insert_with(|| History { base: now.clone(), undo: Vec::new(), redo: Vec::new() });
    // Un état devient une étape d'historique quand l'édition est finie (souris relâchée, plus de
    // saisie dans un champ) : un déplacement ou un mot tapé = une seule étape.
    let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
    if !busy && now != h.base {
        h.undo.push(std::mem::replace(&mut h.base, now.clone()));
        if h.undo.len() > UNDO_MAX {
            h.undo.remove(0);
        }
        h.redo.clear();
    }
    if ctx.egui_wants_keyboard_input() || st.canvas.library.is_some() || st.rename.is_some() {
        return false;
    }
    let (undo, redo, copy_key, cut_key, paste_key, events) = ctx.input(|i| {
        let c = i.modifiers.command;
        (
            c && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
            c && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            c && i.key_pressed(egui::Key::C),
            c && i.key_pressed(egui::Key::X),
            c && i.key_pressed(egui::Key::V),
            i.events.clone(),
        )
    });
    let h = st.history.get_mut(&gid).expect("historique");
    let g = &mut st.graphs[idx];
    if undo || redo {
        let (from, to) = if undo { (&mut h.undo, &mut h.redo) } else { (&mut h.redo, &mut h.undo) };
        let Some(snap) = from.pop() else { return false };
        to.push(std::mem::replace(&mut h.base, snap.clone()));
        snap.apply(g);
        st.canvas.ed.selected.retain(|id| g.node(*id).is_some());
        return true;
    }
    // Copier / couper : la sélection, aussi dans le presse-papiers du système.
    let copy = copy_key || events.iter().any(|e| matches!(e, egui::Event::Copy));
    let cut = cut_key || events.iter().any(|e| matches!(e, egui::Event::Cut));
    if (copy || cut) && !st.canvas.ed.selected.is_empty() {
        let ids: Vec<u64> = st.canvas.ed.selected.iter().copied().collect();
        if let Some(clip) = store::Clip::of(g, &ids) {
            if let Ok(text) = serde_json::to_string(&clip) {
                ctx.copy_text(text);
            }
            st.clipboard = Some(clip);
        }
        if cut {
            g.remove_nodes(&ids);
            st.canvas.ed.selected.clear();
            return true;
        }
        return false;
    }
    // Coller : ce que contient le presse-papiers du système s'il vient du Blueprint, sinon la
    // dernière copie ; sous le pointeur (ou au centre de la vue).
    let pasted = events.iter().find_map(|e| if let egui::Event::Paste(t) = e { Some(store::Clip::parse(t)) } else { None });
    let clip = match pasted {
        Some(Some(c)) => Some(c),
        Some(None) if !paste_key => None,
        _ if paste_key || pasted.is_some() => st.clipboard.clone(),
        _ => None,
    };
    let Some(clip) = clip else { return false };
    let at = ctx.pointer_hover_pos().filter(|p| canvas.contains(*p)).unwrap_or(center);
    let at = canvas::to_graph(center, &g.view, at);
    let ids = g.paste(&clip, [editor::snap(at.x), editor::snap(at.y)]);
    st.canvas.ed.selected = ids.into_iter().collect();
    true
}

/// Ombre intérieure le long des bords de la fenêtre (mode plein écran).
fn inner_shadow(p: &egui::Painter, r: Rect, k: f32) {
    use egui::epaint::Mesh;
    const DEPTH: f32 = 70.0;
    let (dark, mid, clear) = (Color32::from_black_alpha((75.0 * k) as u8), Color32::from_black_alpha((22.0 * k) as u8), Color32::TRANSPARENT);
    let mut m = Mesh::default();
    // Bande d'un bord : `a`→`b` le long du bord, `n` vers l'intérieur ; deux paliers pour adoucir.
    let mut band = |a: Pos2, b: Pos2, n: egui::Vec2| {
        for (from, to, c0, c1) in [(0.0, 0.3, dark, mid), (0.3, 1.0, mid, clear)] {
            let i = m.vertices.len() as u32;
            m.colored_vertex(a + n * DEPTH * from, c0);
            m.colored_vertex(b + n * DEPTH * from, c0);
            m.colored_vertex(b + n * DEPTH * to, c1);
            m.colored_vertex(a + n * DEPTH * to, c1);
            m.add_triangle(i, i + 1, i + 2);
            m.add_triangle(i, i + 2, i + 3);
        }
    };
    band(r.left_top(), r.right_top(), vec2(0.0, 1.0));
    band(r.left_bottom(), r.right_bottom(), vec2(0.0, -1.0));
    band(r.left_top(), r.left_bottom(), vec2(1.0, 0.0));
    band(r.right_top(), r.right_bottom(), vec2(-1.0, 0.0));
    p.add(m);
}

/// Pilule de commandes du graph : liste, zoom, recentrer, plein écran, simuler.
#[allow(clippy::too_many_arguments)]
fn controls(ctx: &egui::Context, pos: Pos2, full: bool, side_hidden: bool, zoom: f32, simulating: bool, active: bool) -> Option<Control> {
    let mut out = None;
    // Calque au premier plan : au-dessus du graph, même agrandi (il ne reçoit pas les clics de
    // la pilule).
    egui::Area::new(egui::Id::unique("wt_bp_controls")).order(egui::Order::Foreground).fixed_pos(pos).fade_in(false).show(ctx, |ui| {
        egui::Frame::new()
            .fill(Color32::from_rgb(0x1A, 0x1A, 0x1A))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x3A, 0x3A, 0x3A)))
            .corner_radius(CornerRadius::same(255))
            .inner_margin(egui::Margin::symmetric(6, 4))
            .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(120) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    {
                        let tip = if side_hidden { t!("Afficher la liste des graphs", "Show the graph list") } else { t!("Masquer la liste des graphs", "Hide the graph list") };
                        let glyph = if side_hidden { "»" } else { "«" };
                        if pill_button(ui, tip, |p, r, c| {
                            p.text(r.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(15.0), c);
                        })
                        .clicked()
                        {
                            out = Some(Control::ToggleSide);
                        }
                        divider(ui);
                    }
                    if pill_button(ui, t!("Zoom arrière", "Zoom out"), |p, r, c| {
                        p.hline(r.center().x - 5.0..=r.center().x + 5.0, r.center().y, Stroke::new(1.6, c));
                    })
                    .clicked()
                    {
                        out = Some(Control::ZoomOut);
                    }
                    let pct = egui::Button::new(RichText::new(format!("{} %", (zoom * 100.0).round())).size(12.0).color(Color32::from_gray(0xB0)))
                        .frame(false)
                        .min_size(vec2(48.0, 28.0));
                    if ui.add(pct).on_hover_text(t!("Revenir à 100 %", "Back to 100 %")).clicked() {
                        out = Some(Control::ZoomReset);
                    }
                    if pill_button(ui, t!("Zoom avant", "Zoom in"), |p, r, c| {
                        let s = Stroke::new(1.6, c);
                        p.hline(r.center().x - 5.0..=r.center().x + 5.0, r.center().y, s);
                        p.vline(r.center().x, r.center().y - 5.0..=r.center().y + 5.0, s);
                    })
                    .clicked()
                    {
                        out = Some(Control::ZoomIn);
                    }
                    if pill_button(ui, t!("Recentrer la vue (Origine)", "Recentre the view (Home)"), |p, r, c| {
                        // Viseur : cercle et quatre traits.
                        let (cc, s) = (r.center(), Stroke::new(1.5, c));
                        p.circle_stroke(cc, 4.0, s);
                        for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                            p.line_segment([cc + d * 5.5, cc + d * 8.0], s);
                        }
                    })
                    .clicked()
                    {
                        out = Some(Control::Recenter);
                    }
                    divider(ui);
                    let tip = if full { t!("Quitter le plein écran (Échap)", "Exit full screen (Esc)") } else { t!("Plein écran", "Full screen") };
                    if pill_button(ui, tip, |p, r, c| fullscreen_icon(p, r.center(), c, full)).clicked() {
                        out = Some(Control::Fullscreen);
                    }
                    divider(ui);
                    if pill_button(ui, t!("Données d'exemple de la simulation", "Simulation sample data"), |p, r, c| draw_sample(p, r.center(), c)).clicked() {
                        out = Some(Control::Sample);
                    }
                    // Actif : le graph s'exécute pour de vrai.
                    if active_switch(ui, active).clicked() {
                        out = Some(Control::Active);
                    }
                    // Simuler : triangle « lecture » dessiné (le caractère ▶ s'afficherait en émoji).
                    let fg = if simulating { Color32::BLACK } else { Color32::from_gray(0xE8) };
                    let sim = egui::Button::new(RichText::new(t!("      Simuler", "      Simulate")).strong().color(fg))
                        .fill(if simulating { canvas::SIM } else { Color32::from_rgb(0x2A, 0x2A, 0x2A) })
                        .corner_radius(CornerRadius::same(255))
                        .min_size(vec2(96.0, 28.0));
                    let sim = ui
                        .add_enabled(!simulating, sim)
                        .on_hover_text(t!("Exécute le graph à blanc, de l'event jusqu'au bout du flux", "Dry-runs the graph, from the event to the end of the flow"));
                    let c = sim.rect.left_center() + vec2(18.0, 0.0);
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, -5.5), c + vec2(6.0, 0.0), c + vec2(-4.0, 5.5)], fg, Stroke::NONE));
                    if sim.clicked() {
                        out = Some(Control::Simulate);
                    }
                });
            });
    });
    out
}

/// Vert des graphs actifs.
const ACTIVE: Color32 = Color32::from_rgb(0x3D, 0xC4, 0x6A);

/// Interrupteur dessiné (centre `c`, échelle `k` : 1 = 28 × 16).
fn draw_switch(p: &egui::Painter, c: Pos2, k: f32, on: bool, ink: Color32) {
    let track = Rect::from_center_size(c, vec2(28.0, 16.0) * k);
    if on {
        p.rect_filled(track, CornerRadius::same(255), ACTIVE);
    } else {
        p.rect_stroke(track, CornerRadius::same(255), Stroke::new(1.4, ink), egui::StrokeKind::Inside);
    }
    let x = if on { track.right() - track.height() / 2.0 } else { track.left() + track.height() / 2.0 };
    p.circle_filled(Pos2::new(x, c.y), track.height() / 2.0 - 3.0 * k, if on { Color32::WHITE } else { ink });
}

/// Interrupteur « Actif » de la pilule.
fn active_switch(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let label = if on { t!("Actif", "Active") } else { t!("Inactif", "Inactive") };
    let font = egui::FontId::proportional(12.5);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(w + 50.0, 28.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if on {
        t!(
            "Ce graph s'exécute pour de vrai à l'arrivée des mails, messages et rendez-vous. Clic : le désactiver.",
            "This graph really runs when mails, messages and events arrive. Click to deactivate it."
        )
    } else {
        t!(
            "Activer ce graph : il s'exécutera pour de vrai à l'arrivée des mails, messages et rendez-vous (« Simuler » reste à blanc).",
            "Activate this graph: it will really run when mails, messages and events arrive (“Simulate” stays a dry run)."
        )
    });
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(14), Color32::from_gray(0x30));
    }
    let ink = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(0xB0) };
    draw_switch(ui.painter(), Pos2::new(r.left() + 22.0, r.center().y), 1.0, on, ink);
    ui.painter().text(Pos2::new(r.left() + 42.0, r.center().y), egui::Align2::LEFT_CENTER, label, font, if on { ACTIVE } else { ink });
    resp
}

/// Bouton rond de la pilule, icône dessinée.
fn pill_button(ui: &mut egui::Ui, tip: &str, icon: impl FnOnce(&egui::Painter, Rect, Color32)) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(14), Color32::from_gray(0x30));
    }
    icon(ui.painter(), r, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(0xB0) });
    resp
}

fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 28.0), Sense::hover());
    ui.painter().vline(r.center().x, r.center().y - 8.0..=r.center().y + 8.0, Stroke::new(1.0, Color32::from_gray(0x3A)));
}

/// Données d'exemple : une enveloppe.
fn draw_sample(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    let r = Rect::from_center_size(c, vec2(13.0, 9.5));
    p.rect_stroke(r, CornerRadius::same(1), s, egui::StrokeKind::Inside);
    p.add(egui::Shape::line(vec![r.left_top() + vec2(1.0, 1.0), Pos2::new(c.x, c.y + 1.0), r.right_top() + vec2(-1.0, 1.0)], s));
}

/// Croix de fermeture.
fn draw_close(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.5, color);
    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
    p.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], s);
}

/// Quatre coins : vers l'extérieur pour agrandir, vers l'intérieur pour revenir.
fn fullscreen_icon(p: &egui::Painter, c: Pos2, color: Color32, exit: bool) {
    let s = Stroke::new(1.6, color);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let (corner, arm) = if exit { (c + vec2(sx * 2.0, sy * 2.0), 3.5) } else { (c + vec2(sx * 6.0, sy * 6.0), -3.5) };
        p.line_segment([corner, corner + vec2(sx * arm, 0.0)], s);
        p.line_segment([corner, corner + vec2(0.0, sy * arm)], s);
    }
}

/// Journal de simulation, sous le graph (lignes qui apparaissent au fil de la simulation).
/// Journal dans son propre calque, au-dessus du graph (même agrandi) : la molette y fait
/// défiler le journal au lieu de zoomer le graph.
fn log_panel(ui: &mut egui::Ui, rect: Rect, st: &mut BlueprintState, now: f64) {
    egui::Area::new(egui::Id::unique("wt_bp_log_float")).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false).fade_in(false).show(ui.ctx(), |ui| {
        let (r, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
        log_body(ui, r, st, now);
    });
}

fn log_body(ui: &mut egui::Ui, rect: Rect, st: &mut BlueprintState, now: f64) {
    ui.painter().rect(rect, CornerRadius::same(10), Color32::from_rgb(0x1A, 0x1A, 0x1A), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
    // Les lignes apparaissent au rythme des étapes de la simulation.
    let shown = st.sim_progress(now).map(|p| if p.2 { p.0 } else { usize::MAX }).unwrap_or(usize::MAX);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.horizontal(|ui| {
            // Simulation (à blanc) ou exécutions réelles des graphs actifs.
            let runs = st.engine.runs.len();
            let tabs = [(false, t!("Simulation", "Simulation").to_string()), (true, tf!("Exécutions réelles ({runs})", "Real runs ({runs})"))];
            for (real, label) in tabs {
                let on = st.log_runs == real;
                let text = RichText::new(label).small().strong().color(if on { Color32::from_gray(0xE8) } else { Color32::from_gray(0x80) });
                if ui.add(egui::Button::new(text).frame(on).fill(Color32::from_gray(0x2A))).clicked() {
                    st.log_runs = real;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill_button(ui, t!("Fermer", "Close"), |p, r, c| draw_close(p, r.center(), c)).clicked() {
                    st.log_open = false;
                }
                if ui.small_button(t!("Effacer", "Clear")).clicked() {
                    if st.log_runs {
                        st.engine.runs.clear();
                    } else {
                        st.trace = None;
                        st.sim_started = None;
                    }
                }
            });
        });
        ui.add_space(2.0);
        if st.log_runs {
            egui::ScrollArea::vertical().id_salt("wt_bp_runs").auto_shrink([false, false]).show(ui, |ui| {
                if st.engine.runs.is_empty() {
                    ui.label(
                        RichText::new(t!(
                            "Aucune exécution réelle depuis le lancement. Active un graph (interrupteur « Actif ») : il tournera à l'arrivée des prochains mails, messages ou rendez-vous.",
                            "No real run since launch. Activate a graph (“Active” switch): it will run when the next mails, messages or events arrive."
                        ))
                        .small()
                        .color(Color32::from_gray(0x70)),
                    );
                }
                // Les plus récentes en haut.
                for r in &st.engine.runs {
                    ui.label(RichText::new(format!("[{}] {}", r.when(), r.graph)).monospace().size(12.0).strong().color(ACTIVE));
                    for l in &r.lines {
                        let color = if l.warn { canvas::SIM } else { Color32::from_gray(0xB8) };
                        ui.label(RichText::new(format!("    {}", l.text)).monospace().size(12.0).color(color));
                    }
                }
            });
            return;
        }
        egui::ScrollArea::vertical().id_salt("wt_bp_log").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let Some(trace) = &st.trace else {
                ui.label(RichText::new(t!("Lance « Simuler » pour voir le déroulé du graph.", "Press “Simulate” to see the graph run.")).small().color(Color32::from_gray(0x70)));
                return;
            };
            let at = st.sim_at.map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default();
            for l in trace.lines.iter().filter(|l| l.step <= shown) {
                let color = if l.warn { canvas::SIM } else { Color32::from_gray(0xB8) };
                ui.label(RichText::new(format!("[{at}] {}", l.text)).monospace().size(12.0).color(color));
            }
        });
    });
}

/// Fenêtre des données d'exemple : le faux mail, message et rendez-vous de la simulation.
/// Panneau flottant sous la pilule de commandes, au premier plan.
fn sample_window(ctx: &egui::Context, st: &mut BlueprintState) {
    if !st.sample_open {
        return;
    }
    let Some(id) = st.selected.clone() else { return };
    let pos = st.sample_pos;
    let Some(g) = st.graphs.iter_mut().find(|g| g.id == id) else { return };
    let before = g.sample.clone();
    let s = &mut g.sample;
    let mut close = false;
    let max_h = (ctx.content_rect().bottom() - pos.y - 16.0).max(200.0);
    egui::Area::new(egui::Id::unique("wt_bp_sample")).order(egui::Order::Foreground).fixed_pos(pos).fade_in(false).show(ctx, |ui| {
        egui::Frame::new()
            .fill(Color32::from_rgb(0x1A, 0x1A, 0x1A))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x3A, 0x3A, 0x3A)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::same(12))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(160) })
            .show(ui, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t!("Données d'exemple", "Sample data")).strong().color(Color32::from_gray(0xE8)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if pill_button(ui, t!("Fermer", "Close"), |p, r, c| draw_close(p, r.center(), c)).clicked() {
                            close = true;
                        }
                    });
                });
                ui.label(
                    RichText::new(t!(
                        "« Simuler » fait comme si ce mail, ce message, ce rendez-vous et ce rappel arrivaient. Rien n'est envoyé.",
                        "“Simulate” acts as if this mail, message, event and reminder arrived. Nothing is sent."
                    ))
                    .small()
                    .color(Color32::from_gray(0x90)),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical().id_salt("wt_bp_sample_scroll").max_height(max_h - 90.0).show(ui, |ui| {
                    let field = |ui: &mut egui::Ui, label: &str, v: &mut String| {
                        ui.horizontal(|ui| {
                            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(label).size(12.5).color(Color32::from_gray(0xA8))));
                            ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY));
                        });
                    };
                    let section = |ui: &mut egui::Ui, title: &str| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(title).size(12.5).strong().color(Color32::from_gray(0xD8)));
                    };
                    section(ui, "Mail");
                    field(ui, t!("Expéditeur", "Sender"), &mut s.mail_from_name);
                    field(ui, t!("Adresse", "Address"), &mut s.mail_from_email);
                    field(ui, t!("Reçu via (groupe)", "Via (group)"), &mut s.mail_via);
                    field(ui, t!("Objet", "Subject"), &mut s.mail_subject);
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(t!("Contenu", "Body")).size(12.5).color(Color32::from_gray(0xA8))));
                        ui.add(egui::TextEdit::multiline(&mut s.mail_body).desired_rows(3).desired_width(f32::INFINITY));
                    });
                    field(ui, t!("Destinataires", "Recipients"), &mut s.mail_to);
                    section(ui, t!("Message Google Chat", "Google Chat message"));
                    field(ui, t!("Auteur", "Author"), &mut s.chat_author);
                    field(ui, t!("Adresse", "Address"), &mut s.chat_author_email);
                    field(ui, "Conversation", &mut s.chat_space);
                    field(ui, t!("Texte", "Text"), &mut s.chat_text);
                    section(ui, t!("Rendez-vous", "Event"));
                    field(ui, t!("Titre", "Title"), &mut s.event_title);
                    field(ui, t!("Début", "Start"), &mut s.event_start);
                    field(ui, t!("Lieu", "Location"), &mut s.event_location);
                    section(ui, t!("Rappel", "Reminder"));
                    field(ui, t!("Titre", "Title"), &mut s.reminder_title);
                    field(ui, t!("Heure", "Time"), &mut s.reminder_due);
                    ui.add_space(8.0);
                    if ui.button(t!("Réinitialiser", "Reset")).clicked() {
                        *s = store::Sample::default();
                    }
                });
            });
    });
    let changed = g.sample != before;
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.sample_open = false;
    }
    if changed {
        st.mark(&id, true);
    }
}

fn delete_dialog(ctx: &egui::Context, st: &mut BlueprintState) {
    let Some(id) = st.confirm_delete.clone() else { return };
    let name = st.graphs.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default();
    let mut close = false;
    let r = egui::Modal::new(egui::Id::unique("wt_bp_delete")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(RichText::new(t!("Supprimer le graph ?", "Delete the graph?")).size(16.0).strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new(tf!("« {name} » sera supprimé définitivement de ton ordinateur.", "“{name}” will be permanently deleted from your computer.")).color(theme::TEXT_MUTED()),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new(t!("Supprimer", "Delete")).color(theme::TODAY())).clicked() {
                st.delete(&id);
                close = true;
            }
            if ui.button(t!("Annuler", "Cancel")).clicked() {
                close = true;
            }
        });
    });
    if close || r.should_close() {
        st.confirm_delete = None;
    }
}

#[cfg(test)]
mod time_tests {
    use super::next_time;
    use chrono::TimeZone;

    #[test]
    fn next_time_is_today_or_tomorrow() {
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 25, 10, 0, 0).unwrap();
        let at = |t: &str| next_time(t, now).map(|ms| chrono::Local.timestamp_millis_opt(ms).unwrap().format("%d %H:%M").to_string());
        assert_eq!(at("14:30").as_deref(), Some("25 14:30"));
        assert_eq!(at("9h15").as_deref(), Some("26 09:15"));
        assert_eq!(at("18").as_deref(), Some("25 18:00"));
        assert_eq!(at("n'importe quoi"), None);
    }
}
