//! Onglet Fichiers : explorateur Google Drive (Mon Drive, partagés, récents, suivis, Drives partagés),
//! recherche, aperçu et ouverture des fichiers.
//!
//! Comme pour Gmail : chaque dossier n'est demandé qu'une fois puis gardé en mémoire (relu au plus
//! toutes les 2 minutes), 100 éléments par requête avec « Charger plus », champs limités au strict
//! nécessaire, toutes les requêtes passent par le limiteur et les reprises de la session.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::google::drive::{self, DriveEvent, DriveFile, SharedDrive, Source};
use crate::google::{Account, Event, Status};
use crate::liquid::Liquid;
use crate::prefs::Prefs;
use crate::tabs::mail::inline::{self, ImgState, InlineImages};
use crate::tabs::mail::view::one_line;
use crate::{theme, widgets};

const REFRESH: Duration = Duration::from_secs(120);
const SIDE_W: f32 = 230.0;
const DETAIL_W: f32 = 300.0;

#[derive(Default)]
pub struct DriveState {
    pub source: Option<Source>,
    back: Vec<Source>,
    listings: HashMap<Source, (Vec<DriveFile>, Option<String>, Instant)>,
    loading: HashSet<Source>,
    drives: Vec<SharedDrive>,
    drives_requested: bool,
    /// Chemin du dossier affiché (identifiant, nom).
    paths: HashMap<String, Vec<(String, String)>>,
    pub selected: Option<DriveFile>,
    query: String,
    by_date: bool,
    grid: bool,
    thumbs: InlineImages,
    pub need_consent: bool,
    pub api_disabled: bool,
    notice: Option<(String, bool, Instant)>,
}

fn run(acc: &Account, f: impl FnOnce(&crate::google::Session) -> Result<DriveEvent, String> + Send + 'static) {
    acc.run(move |s| Ok(Event::Drive(f(s).unwrap_or_else(DriveEvent::Error))));
}

impl DriveState {
    pub fn on_ready(&mut self) {
        *self = DriveState { by_date: self.by_date, grid: self.grid, ..Default::default() };
    }

    pub fn handle(&mut self, ev: DriveEvent, acc: &Account) {
        match ev {
            DriveEvent::Listing { source, files, next, append } => {
                self.loading.remove(&source);
                self.need_consent = false;
                self.api_disabled = false;
                let entry = self.listings.entry(source).or_insert_with(|| (Vec::new(), None, Instant::now()));
                if append {
                    entry.0.extend(files);
                } else {
                    entry.0 = files;
                }
                entry.1 = next;
                entry.2 = Instant::now();
            }
            DriveEvent::Drives(d) => self.drives = d,
            DriveEvent::Path { folder, path } => {
                self.paths.insert(folder, path);
            }
            DriveEvent::Thumb { key, image } => self.thumbs.on_loaded(key, image),
            DriveEvent::Downloaded { path, open } => {
                if open {
                    if let Err(e) = open::that(&path) {
                        self.error(format!("Impossible d'ouvrir le fichier : {e}"));
                    }
                } else {
                    self.info(&format!("Enregistré : {}", path.display()));
                }
            }
            DriveEvent::Found { .. } => {}
            DriveEvent::Error(e) => {
                self.loading.clear();
                let low = e.to_lowercase();
                if low.contains("insufficient") || low.contains("scope") {
                    self.need_consent = true;
                } else if low.contains("has not been used") || low.contains("accessnotconfigured") || low.contains("service_disabled") || low.contains("is disabled") {
                    self.api_disabled = true;
                } else {
                    self.error(e);
                }
            }
        }
        let _ = acc;
    }

    fn current(&self) -> Source {
        self.source.clone().unwrap_or(Source::MyDrive)
    }

    /// Change de dossier / vue (mémorise la précédente pour « retour »).
    pub fn go(&mut self, acc: &Account, s: Source) {
        let cur = self.current();
        if cur != s {
            self.back.push(cur);
            if self.back.len() > 50 {
                self.back.remove(0);
            }
        }
        self.source = Some(s.clone());
        self.selected = None;
        self.ensure(acc, &s, false);
        if let Source::Folder(id) = &s
            && !self.paths.contains_key(id)
        {
            let id = id.clone();
            run(acc, move |sess| Ok(DriveEvent::Path { path: drive::path_of(sess, &id)?, folder: id }));
        }
    }

    fn ensure(&mut self, acc: &Account, s: &Source, force: bool) {
        let fresh = self.listings.get(s).is_some_and(|(_, _, t)| t.elapsed() < REFRESH);
        if (fresh && !force) || self.loading.contains(s) {
            return;
        }
        self.loading.insert(s.clone());
        let src = s.clone();
        run(acc, move |sess| {
            let (files, next) = drive::list(sess, &src, None)?;
            Ok(DriveEvent::Listing { source: src, files, next, append: false })
        });
    }

    fn more(&mut self, acc: &Account) {
        let s = self.current();
        let Some(token) = self.listings.get(&s).and_then(|l| l.1.clone()) else { return };
        if !self.loading.insert(s.clone()) {
            return;
        }
        run(acc, move |sess| {
            let (files, next) = drive::list(sess, &s, Some(&token))?;
            Ok(DriveEvent::Listing { source: s, files, next, append: true })
        });
    }

    /// Affiche un fichier trouvé ailleurs (recherche globale) : dossier parent, fichier sélectionné.
    pub fn reveal(&mut self, acc: &Account, f: DriveFile) {
        match f.parents.first() {
            Some(p) => self.go(acc, Source::Folder(p.clone())),
            None => self.go(acc, Source::Search(f.name.clone())),
        }
        if f.is_folder() {
            self.go(acc, Source::Folder(f.id.clone()));
        } else {
            self.selected = Some(f);
        }
    }

    /// Ouvre : dossier = y entrer ; document Google = navigateur ; autre = téléchargement puis
    /// application par défaut (navigateur en dernier recours).
    pub fn open(&mut self, acc: &Account, f: &DriveFile) {
        if f.is_folder() {
            return self.go(acc, Source::Folder(f.id.clone()));
        }
        if f.is_google() {
            if let Some(u) = &f.web_link {
                let _ = webbrowser::open(u);
            }
            return;
        }
        self.download(acc, f, true);
    }

    /// Ouvre avec l'application du PC (export Office pour les documents Google).
    pub fn open_local(&mut self, acc: &Account, f: &DriveFile) {
        self.download(acc, f, true);
    }

    fn download(&mut self, acc: &Account, f: &DriveFile, open: bool) {
        let target = if open {
            None
        } else {
            let name = match drive::export_format(&f.mime) {
                Some((_, ext)) if f.is_google() => format!("{}.{ext}", f.name),
                _ => f.name.clone(),
            };
            match rfd::FileDialog::new().set_file_name(&name).save_file() {
                Some(p) => Some(p),
                None => return,
            }
        };
        self.info(&format!("Téléchargement de « {} »…", f.name));
        let (file, web) = (f.clone(), f.web_link.clone());
        run(acc, move |s| match drive::download(s, &file) {
            Ok((data, name)) => {
                let path = match target {
                    Some(p) => p,
                    None => {
                        let dir = std::env::temp_dir().join("WorkTable").join("drive");
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                        dir.join(sanitize(&name))
                    }
                };
                std::fs::write(&path, data).map_err(|e| e.to_string())?;
                Ok(DriveEvent::Downloaded { path, open })
            }
            Err(e) => {
                if let Some(u) = web {
                    let _ = webbrowser::open(&u);
                }
                Err(e)
            }
        });
    }

    fn request_thumb(&mut self, acc: &Account, f: &DriveFile) {
        let Some(url) = f.thumbnail.clone() else { return };
        let key = format!("drive:{}", f.id);
        if !self.thumbs.start(&key) {
            return;
        }
        run(acc, move |s| {
            let image = drive::thumbnail(s, &url).and_then(|d| inline::decode(&d));
            Ok(DriveEvent::Thumb { key, image })
        });
    }

    /// Ouvre l'aperçu en grand et charge une version plus nette (remplacée dès qu'elle arrive).
    fn view_large(&mut self, ctx: &egui::Context, acc: &Account, f: &DriveFile, tex: egui::TextureHandle) {
        let key = format!("drive-hd:{}", f.id);
        let open = match &f.web_link {
            Some(u) => crate::lightbox::OpenWith::Url(u.clone()),
            None => crate::lightbox::OpenWith::None,
        };
        let hd = match self.thumbs.get(ctx, &key) {
            Some(ImgState::Ready { texture: Some(t), .. }) => t.clone(),
            _ => tex,
        };
        crate::lightbox::open(ctx, key.clone(), hd, f.name.clone(), open);
        let Some(url) = f.thumbnail.clone() else { return };
        if self.thumbs.start(&key) {
            run(acc, move |s| {
                let image = drive::thumbnail_sized(s, &url, 2000).and_then(|d| inline::decode(&d));
                Ok(DriveEvent::Thumb { key, image })
            });
        }
    }

    fn info(&mut self, t: &str) {
        self.notice = Some((t.to_string(), false, Instant::now()));
    }

    fn error(&mut self, t: String) {
        self.notice = Some((t, true, Instant::now()));
    }
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    if s.trim().is_empty() { "fichier".into() } else { s }
}

// --- Affichage ---------------------------------------------------------------------------

pub fn show(ui: &mut egui::Ui, st: &mut DriveState, acc: &mut Account, prefs: &mut Prefs) {
    // Visionneuse : la version nette remplace l'aperçu dès qu'elle est chargée.
    if let Some(k) = crate::lightbox::current().filter(|k| k.starts_with("drive-hd:"))
        && let Some(ImgState::Ready { texture: Some(t), .. }) = st.thumbs.get(ui.ctx(), &k)
    {
        crate::lightbox::upgrade(&k, t.clone());
    }
    crate::widgets::page_header(ui, "Fichiers", "Google Drive");

    if acc.session().is_none() {
        card(ui, "Google Drive", "Connecte ton compte Google pour parcourir et ouvrir tes fichiers Drive.", |ui| {
            crate::tabs::settings::google_account_form(ui, acc);
        });
        return;
    }
    if st.need_consent {
        card(ui, "Autoriser Google Drive", "WorkTable n'a pas encore l'autorisation de lire tes fichiers Drive. Reconnecte-toi une fois pour l'accorder.", |ui| {
            if acc.status == Status::Connecting {
                ui.label("Termine l'autorisation dans le navigateur…");
            } else if ui.button("Autoriser l'accès à Google Drive").clicked() {
                st.need_consent = false;
                acc.connect();
            }
        });
        return;
    }
    if st.api_disabled {
        card(ui, "Activer l'API Google Drive", "L'API Google Drive n'est pas activée dans ton projet Google Cloud. Active-la, attends une minute, puis réessaie.", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Ouvrir la page d'activation").clicked() {
                    let _ = webbrowser::open("https://console.cloud.google.com/apis/library/drive.googleapis.com");
                }
                if ui.button("Réessayer").clicked() {
                    st.api_disabled = false;
                    st.listings.clear();
                }
            });
        });
        return;
    }

    let acc_ref: &Account = acc;
    if st.source.is_none() {
        st.go(acc_ref, Source::MyDrive);
        st.back.clear();
    }
    if !st.drives_requested {
        st.drives_requested = true;
        run(acc_ref, |s| Ok(DriveEvent::Drives(drive::shared_drives(s).unwrap_or_default())));
    }
    let cur = st.current();
    st.ensure(acc_ref, &cur, false);
    shortcuts(ui, st, acc_ref);

    // Colonnes : navigation (masquable, rebond élastique) | liste | détail.
    let k = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("wt_drive_side"), !prefs.files_side_hidden, 0.28, |t| crate::prefs::Curve::Elastic.apply(t));
    let side_w = if SIDE_W * k < 24.0 { 0.0 } else { SIDE_W * k };
    let detail_w = if st.selected.is_some() { DETAIL_W } else { 0.0 };
    let gap = ui.spacing().item_spacing.x;
    let center_w = (ui.available_width() - side_w - detail_w - if side_w > 0.0 { gap } else { 0.0 } - if detail_w > 0.0 { gap } else { 0.0 }).max(260.0);
    let mut cols: Vec<(usize, f32)> = Vec::new();
    if side_w > 0.0 {
        cols.push((0, side_w));
    }
    cols.push((1, center_w));
    if detail_w > 0.0 {
        cols.push((2, detail_w));
    }
    let fixed: Vec<f32> = cols.iter().take(cols.len() - 1).map(|c| c.1).collect();
    let mut toggle_side = false;
    widgets::pane_columns(ui, &fixed, |i, ui| match cols[i].0 {
        0 => {
            let inner = ui.available_rect_before_wrap();
            let margins = side_w - inner.width();
            let content = Rect::from_min_size(inner.min, vec2((SIDE_W - margins).max(inner.width()), inner.height()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(*ui.layout()));
            child.set_clip_rect(inner.expand(2.0).intersect(ui.clip_rect()));
            child.multiply_opacity(k.clamp(0.0, 1.0));
            side(&mut child, st, acc_ref);
            ui.allocate_rect(inner, Sense::hover());
        }
        1 => center(ui, st, acc_ref, prefs.files_side_hidden, &mut toggle_side),
        _ => detail(ui, st, acc_ref),
    });
    if toggle_side {
        prefs.files_side_hidden = !prefs.files_side_hidden;
    }
    toast(ui, st);
}

fn card(ui: &mut egui::Ui, title: &str, text: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, title, 0.0, |ui| {
                ui.label(RichText::new(text).color(theme::TEXT_MUTED()));
                ui.add_space(12.0);
                body(ui);
            });
        });
    });
}

fn shortcuts(ui: &mut egui::Ui, st: &mut DriveState, acc: &Account) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (back, esc) = ui.input(|i| (i.key_pressed(Key::Backspace) || (i.modifiers.alt && i.key_pressed(Key::ArrowLeft)), i.key_pressed(Key::Escape)));
    if esc {
        st.selected = None;
    }
    if back && let Some(s) = st.back.pop() {
        st.source = Some(s.clone());
        st.selected = None;
        st.ensure(acc, &s, false);
    }
}

fn side(ui: &mut egui::Ui, st: &mut DriveState, acc: &Account) {
    let cur = st.current();
    let mut hl = Liquid::begin(ui, "drive_nav");
    let mut go = None;
    for (label, src, icon) in
        [("Mon Drive", Source::MyDrive, "🗂️"), ("Partagés avec moi", Source::Shared, "👥"), ("Récents", Source::Recent, "🕘"), ("Suivis", Source::Starred, "⭐")]
    {
        let active = cur == src || (src == Source::MyDrive && matches!(cur, Source::Folder(_)));
        if nav_row(ui, icon, label, active, &mut hl).clicked() {
            go = Some(src);
        }
    }
    if !st.drives.is_empty() {
        ui.add_space(10.0);
        ui.label(RichText::new("DRIVES PARTAGÉS").small().strong().color(theme::TEXT_MUTED()));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("drive_shared_drives").auto_shrink([false, false]).show(ui, |ui| {
            for d in st.drives.clone() {
                let src = Source::SharedDrive(d.id.clone());
                if nav_row(ui, "🏢", &d.name, cur == src, &mut hl).clicked() {
                    go = Some(src);
                }
            }
        });
    }
    hl.end(ui, crate::theme::gray(0x2A), 8.0);
    if let Some(s) = go {
        st.go(acc, s);
    }
}

fn nav_row(ui: &mut egui::Ui, icon: &str, text: &str, active: bool, hl: &mut Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if active {
        hl.target(rect, text);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), crate::theme::overlay(7));
    }
    crate::emoji_img::paint(p, Rect::from_center_size(Pos2::new(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)), icon, theme::TEXT());
    let color = if active { theme::TEXT() } else { theme::TEXT_MUTED() };
    let g = one_line(p, text, FontId::proportional(14.0), color, rect.width() - 40.0);
    p.galley(Pos2::new(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, color);
    resp
}

fn title_of(st: &DriveState, s: &Source) -> String {
    match s {
        Source::MyDrive => "Mon Drive".into(),
        Source::Shared => "Partagés avec moi".into(),
        Source::Recent => "Récents".into(),
        Source::Starred => "Suivis".into(),
        Source::SharedDrive(id) => st.drives.iter().find(|d| d.id == *id).map(|d| d.name.clone()).unwrap_or_else(|| "Drive partagé".into()),
        Source::Search(q) => format!("Résultats pour « {q} »"),
        Source::Folder(id) => st.paths.get(id).and_then(|p| p.last()).map(|x| x.1.clone()).unwrap_or_else(|| "Dossier".into()),
    }
}

fn center(ui: &mut egui::Ui, st: &mut DriveState, acc: &Account, side_hidden: bool, toggle_side: &mut bool) {
    let cur = st.current();
    // Ligne 1 : panneau, retour, recherche, actualiser.
    ui.horizontal(|ui| {
        if ui.button(if side_hidden { "»" } else { "«" }).on_hover_text("Afficher / masquer la navigation").clicked() {
            *toggle_side = true;
        }
        if ui.add_enabled(!st.back.is_empty(), egui::Button::new("‹")).on_hover_text("Retour (Retour arrière)").clicked()
            && let Some(s) = st.back.pop()
        {
            st.source = Some(s.clone());
            st.selected = None;
            st.ensure(acc, &s, false);
        }
        let search =
            ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Rechercher dans Drive (nom ou contenu) — Entrée").desired_width((ui.available_width() - 44.0).max(120.0)));
        if search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !st.query.trim().is_empty() {
            let q = st.query.trim().to_string();
            st.go(acc, Source::Search(q));
        }
        if ui.button("⟳").on_hover_text("Actualiser").clicked() {
            st.ensure(acc, &cur, true);
        }
    });
    ui.add_space(8.0);

    // Ligne 2 : fil d'Ariane à gauche, tri et affichage à droite.
    ui.horizontal(|ui| {
        let crumbs_w = (ui.available_width() - 250.0).max(120.0);
        ui.allocate_ui_with_layout(vec2(crumbs_w, 28.0), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut go = None;
            match &cur {
                Source::Folder(id) => {
                    if ui.add(egui::Button::new(RichText::new("Mon Drive").color(theme::TEXT_MUTED())).frame(false)).clicked() {
                        go = Some(Source::MyDrive);
                    }
                    let path = st.paths.get(id).cloned().unwrap_or_default();
                    for (i, (fid, name)) in path.iter().enumerate() {
                        ui.label(RichText::new("›").color(theme::TEXT_FAINT()));
                        let last = i + 1 == path.len();
                        let t = RichText::new(name).color(if last { theme::TEXT() } else { theme::TEXT_MUTED() });
                        let t = if last { t.strong() } else { t };
                        if ui.add(egui::Button::new(t).frame(false)).clicked() && !last {
                            go = Some(Source::Folder(fid.clone()));
                        }
                    }
                }
                s => {
                    let title = title_of(st, s);
                    ui.add(egui::Label::new(RichText::new(title).font(theme::title_font(16.0)).color(theme::TEXT())).truncate());
                }
            }
            if st.loading.contains(&cur) {
                ui.add(egui::Spinner::new().size(14.0));
            }
            if let Some(s) = go {
                st.go(acc, s);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut grid = st.grid;
            if widgets::segmented(ui, "drive_view", &[(true, "Grille"), (false, "Liste")], &mut grid) {
                st.grid = grid;
            }
            let mut by_date = st.by_date;
            if widgets::segmented(ui, "drive_sort", &[(true, "Modifié"), (false, "Nom")], &mut by_date) {
                st.by_date = by_date;
            }
        });
    });
    ui.add_space(6.0);

    let mut files: Vec<DriveFile> = st.listings.get(&cur).map(|l| l.0.clone()).unwrap_or_default();
    let has_more = st.listings.get(&cur).is_some_and(|l| l.1.is_some());
    // Tri : dossiers d'abord, puis nom ou date (les vues Récents / Partagés gardent l'ordre de Drive).
    if !matches!(cur, Source::Recent | Source::Shared | Source::Search(_)) || st.by_date {
        files.sort_by(|a, b| {
            b.is_folder()
                .cmp(&a.is_folder())
                .then_with(|| if st.by_date { b.modified.cmp(&a.modified) } else { crate::tabs::mail::contacts::fold(&a.name).cmp(&crate::tabs::mail::contacts::fold(&b.name)) })
        });
    }
    if files.is_empty() {
        if st.loading.contains(&cur) {
            for r in [0.8, 0.6, 0.7, 0.5, 0.65] {
                widgets::ghost_row(ui, r);
                ui.add_space(10.0);
            }
        } else {
            ui.add_space(40.0);
            let hint = if matches!(cur, Source::Search(_)) { "Aucun fichier ne correspond" } else { "Ce dossier est vide" };
            widgets::empty_state(ui, "Aucun fichier", hint);
        }
        return;
    }

    let mut open = None;
    let mut select = None;
    let mut menu_action: Option<(DriveFile, u8)> = None;
    egui::ScrollArea::vertical().id_salt(("drive_list", format!("{cur:?}"))).auto_shrink([false, false]).show(ui, |ui| {
        if st.grid {
            let tile = vec2(150.0, 160.0);
            let per_row = ((ui.available_width() + 10.0) / (tile.x + 10.0)).floor().max(1.0) as usize;
            for row in files.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for f in row {
                        if !f.is_folder() {
                            st.request_thumb(acc, f);
                        }
                        let r = grid_tile(ui, st, f, tile);
                        handle(&r, f, &mut open, &mut select, &mut menu_action);
                    }
                });
                ui.add_space(10.0);
            }
        } else {
            list_header(ui);
            let mut hl = Liquid::begin(ui, "drive_list");
            for f in &files {
                let active = st.selected.as_ref().is_some_and(|s| s.id == f.id);
                let r = list_row(ui, f, active, &mut hl);
                handle(&r, f, &mut open, &mut select, &mut menu_action);
            }
            hl.end(ui, crate::theme::gray(0x2A), 8.0);
        }
        if has_more {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                if st.loading.contains(&cur) {
                    ui.add(egui::Spinner::new().size(14.0));
                } else if ui.button("Charger plus").clicked() {
                    st.more(acc);
                }
            });
        }
    });
    if let Some(f) = select {
        st.selected = Some(f);
    }
    if let Some(f) = open {
        st.open(acc, &f);
    }
    if let Some((f, a)) = menu_action {
        match a {
            0 => st.open(acc, &f),
            1 => {
                if let Some(u) = &f.web_link {
                    let _ = webbrowser::open(u);
                }
            }
            2 => st.open_local(acc, &f),
            3 => st.download(acc, &f, false),
            _ => {
                if let Some(u) = &f.web_link {
                    ui.ctx().copy_text(u.clone());
                    st.info("Lien copié");
                }
            }
        }
    }
}

/// Clic = sélection, double-clic = ouvrir, clic droit = menu.
fn handle(r: &egui::Response, f: &DriveFile, open: &mut Option<DriveFile>, select: &mut Option<DriveFile>, menu: &mut Option<(DriveFile, u8)>) {
    if r.double_clicked() {
        *open = Some(f.clone());
    } else if r.clicked() {
        *select = Some(f.clone());
    }
    r.context_menu(|ui| {
        ui.set_min_width(220.0);
        let label = if f.is_folder() {
            "Ouvrir le dossier"
        } else if f.is_google() {
            "Ouvrir dans le navigateur"
        } else {
            "Ouvrir"
        };
        if ui.button(label).clicked() {
            *menu = Some((f.clone(), 0));
            ui.close();
        }
        if !f.is_folder() {
            if !f.is_google() && ui.button("Ouvrir dans le navigateur").clicked() {
                *menu = Some((f.clone(), 1));
                ui.close();
            }
            if f.is_google() && drive::export_format(&f.mime).is_some() && ui.button("Ouvrir avec une application du PC").clicked() {
                *menu = Some((f.clone(), 2));
                ui.close();
            }
            if (!f.is_google() || drive::export_format(&f.mime).is_some()) && ui.button("Télécharger…").clicked() {
                *menu = Some((f.clone(), 3));
                ui.close();
            }
        }
        if ui.button("Copier le lien").clicked() {
            *menu = Some((f.clone(), 4));
            ui.close();
        }
    });
}

fn list_header(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let p = ui.painter();
    let cols = columns(r);
    for (x, t) in [(r.left() + 44.0, "NOM"), (cols.0, "PROPRIÉTAIRE"), (cols.1, "MODIFIÉ"), (cols.2, "TAILLE")] {
        p.text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, t, FontId::proportional(10.5), theme::TEXT_FAINT());
    }
}

/// Positions des colonnes Propriétaire / Modifié / Taille (masquées si la place manque).
fn columns(r: Rect) -> (f32, f32, f32) {
    let w = r.width();
    if w > 720.0 {
        (r.right() - 380.0, r.right() - 200.0, r.right() - 70.0)
    } else if w > 520.0 {
        (f32::INFINITY, r.right() - 190.0, r.right() - 70.0)
    } else {
        (f32::INFINITY, r.right() - 110.0, f32::INFINITY)
    }
}

fn list_row(ui: &mut egui::Ui, f: &DriveFile, active: bool, hl: &mut Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if active {
        hl.target(rect, &f.id);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), crate::theme::overlay(6));
    }
    file_icon(p, Rect::from_center_size(Pos2::new(rect.left() + 20.0, rect.center().y), vec2(26.0, 26.0)), f);
    let (c_owner, c_mod, c_size) = columns(rect);
    let name_w = c_owner.min(c_mod) - rect.left() - 56.0;
    let g = one_line(p, &f.name, FontId::proportional(13.5), theme::TEXT(), name_w);
    p.galley(Pos2::new(rect.left() + 44.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT());
    let muted = |x: f32, t: &str, w: f32| {
        if x.is_finite() {
            let g = one_line(p, t, FontId::proportional(12.0), theme::TEXT_MUTED(), w);
            p.galley(Pos2::new(x, rect.center().y - g.size().y / 2.0), g, theme::TEXT_MUTED());
        }
    };
    muted(c_owner, if f.owner.is_empty() { "—" } else { &f.owner }, 170.0);
    muted(c_mod, &modified(f.modified), 120.0);
    muted(c_size, &f.size.map(size).unwrap_or_else(|| "—".into()), 64.0);
    if f.starred {
        crate::tabs::mail::view::draw_star(p, Pos2::new(rect.left() + 34.0, rect.top() + 10.0), 4.0, true, theme::TEXT_MUTED());
    }
    resp
}

fn grid_tile(ui: &mut egui::Ui, st: &DriveState, f: &DriveFile, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&f.name);
    let active = st.selected.as_ref().is_some_and(|s| s.id == f.id);
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(10),
        if active {
            crate::theme::gray(0x2A)
        } else if resp.hovered() {
            theme::SURFACE_HI()
        } else {
            theme::SURFACE()
        },
        Stroke::new(1.0, theme::BORDER()),
        egui::StrokeKind::Inside,
    );
    let img_rect = Rect::from_min_max(rect.min + vec2(8.0, 8.0), Pos2::new(rect.right() - 8.0, rect.bottom() - 40.0));
    match st.thumbs.peek(&format!("drive:{}", f.id)) {
        Some(ImgState::Ready { texture: Some(t), .. }) => {
            let s = t.size_vec2();
            let k = (img_rect.width() / s.x).min(img_rect.height() / s.y);
            let r = Rect::from_center_size(img_rect.center(), s * k);
            p.image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        _ => file_icon(p, Rect::from_center_size(img_rect.center(), vec2(52.0, 52.0)), f),
    }
    let g = one_line(p, &f.name, FontId::proportional(12.5), theme::TEXT(), rect.width() - 16.0);
    p.galley(Pos2::new(rect.left() + 8.0, rect.bottom() - 34.0), g, theme::TEXT());
    let g = one_line(p, &modified(f.modified), FontId::proportional(10.5), theme::TEXT_FAINT(), rect.width() - 16.0);
    p.galley(Pos2::new(rect.left() + 8.0, rect.bottom() - 17.0), g, theme::TEXT_FAINT());
    resp
}

/// Pastille du type de fichier (couleurs des applications Google).
fn file_icon(p: &egui::Painter, r: Rect, f: &DriveFile) {
    if f.is_folder() {
        let c = crate::theme::gray(0x8C);
        let w = r.width();
        let body = Rect::from_min_max(r.min + vec2(0.0, w * 0.22), r.max - vec2(0.0, w * 0.1));
        p.rect_filled(Rect::from_min_size(r.min + vec2(0.0, w * 0.12), vec2(w * 0.45, w * 0.2)), CornerRadius::same((w * 0.08) as u8), c);
        p.rect_filled(body, CornerRadius::same((w * 0.1) as u8), c);
        return;
    }
    let (label, color) = match drive::kind_label(f) {
        "Google Docs" | "Word" => ("DOC", Color32::from_rgb(0x2B, 0x6C, 0xD6)),
        "Google Sheets" | "Excel" => ("XLS", Color32::from_rgb(0x1E, 0x8E, 0x4E)),
        "Google Slides" | "PowerPoint" => ("PPT", Color32::from_rgb(0xE0, 0x9A, 0x1E)),
        "Google Forms" => ("FORM", Color32::from_rgb(0x67, 0x3A, 0xB7)),
        "Google Drawings" => ("DRAW", Color32::from_rgb(0xD0, 0x4A, 0x3A)),
        "PDF" => ("PDF", Color32::from_rgb(0xC6, 0x45, 0x3B)),
        "Image" => ("IMG", Color32::from_rgb(0x8A, 0x5C, 0xC8)),
        "Vidéo" => ("VID", Color32::from_rgb(0xC8, 0x3C, 0x6E)),
        "Audio" => ("SON", Color32::from_rgb(0x3F, 0xA7, 0xB5)),
        "Archive" => ("ZIP", Color32::from_rgb(0x7D, 0x6E, 0x55)),
        "Raccourci" => ("↗", crate::theme::gray(0x66)),
        _ => ("", crate::theme::gray(0x55)),
    };
    p.rect_filled(r, CornerRadius::same((r.width() * 0.22) as u8), color);
    let text = if label.is_empty() { f.extension().to_uppercase().chars().take(4).collect::<String>() } else { label.to_string() };
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::editor_font((r.width() * 0.3).clamp(8.0, 16.0), true), Color32::WHITE);
}

fn modified(ms: i64) -> String {
    use chrono::Datelike;
    if ms == 0 {
        return "—".into();
    }
    let Some(d) = Local.timestamp_millis_opt(ms).single() else { return "—".into() };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        return d.format("%H:%M").to_string();
    }
    // Mois abrégé par caractères (et non par octets : « août », « février »…).
    let month: String = crate::tabs::month_fr(d.month()).chars().take(4).collect();
    let mut out = format!("{} {}", d.day(), month.trim_end_matches('.'));
    if d.year() != now.year() {
        out.push_str(&format!(" {}", d.year()));
    }
    out
}

fn size(b: u64) -> String {
    match b {
        b if b < 1024 => format!("{b} o"),
        b if b < 1024 * 1024 => format!("{:.0} Ko", b as f64 / 1024.0),
        b if b < 1024 * 1024 * 1024 => format!("{:.1} Mo", b as f64 / 1048576.0),
        b => format!("{:.1} Go", b as f64 / 1073741824.0),
    }
}

// --- Détail ----------------------------------------------------------------------------------

fn detail(ui: &mut egui::Ui, st: &mut DriveState, acc: &Account) {
    let Some(f) = st.selected.clone() else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new("DÉTAILS").small().strong().color(theme::TEXT_MUTED()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Fermer (Échap)").clicked() {
                st.selected = None;
            }
        });
    });
    ui.add_space(8.0);
    st.request_thumb(acc, &f);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 180.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(10), theme::BG());
    let ctx = ui.ctx().clone();
    match st.thumbs.get(&ctx, &format!("drive:{}", f.id)) {
        Some(ImgState::Ready { texture: Some(t), .. }) => {
            let t = t.clone();
            let s = t.size_vec2();
            let k = ((r.width() - 16.0) / s.x).min((r.height() - 16.0) / s.y);
            ui.painter().image(t.id(), Rect::from_center_size(r.center(), s * k), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            // Clic : l'aperçu en grand.
            if resp.on_hover_text("Agrandir l'aperçu").on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                st.view_large(&ctx, acc, &f, t);
            }
        }
        _ => file_icon(ui.painter(), Rect::from_center_size(r.center(), vec2(64.0, 64.0)), &f),
    }
    ui.add_space(10.0);
    ui.add(egui::Label::new(RichText::new(&f.name).font(theme::title_font(15.0)).color(theme::TEXT())).wrap());
    ui.label(RichText::new(drive::kind_label(&f)).small().color(theme::TEXT_MUTED()));
    ui.add_space(10.0);
    let line = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(k).small().color(theme::TEXT_FAINT())));
            ui.add(egui::Label::new(RichText::new(v).small().color(theme::TEXT())).wrap());
        });
    };
    if !f.owner.is_empty() {
        line(ui, "Propriétaire", f.owner.clone());
    }
    line(ui, "Modifié", format!("{}{}", modified(f.modified), if f.modified_by.is_empty() { String::new() } else { format!(" par {}", f.modified_by) }));
    if let Some(s) = f.size {
        line(ui, "Taille", size(s));
    }
    let mut tags = Vec::new();
    if f.shared {
        tags.push("Partagé");
    }
    if f.starred {
        tags.push("Suivi");
    }
    if !tags.is_empty() {
        line(ui, "Statut", tags.join(" · "));
    }
    ui.add_space(14.0);
    let w = ui.available_width();
    if f.is_folder() {
        if ui.add_sized([w, 30.0], egui::Button::new("Ouvrir le dossier")).clicked() {
            st.open(acc, &f);
        }
    } else {
        if f.is_google() {
            if ui.add_sized([w, 30.0], egui::Button::new("Ouvrir dans le navigateur")).clicked() {
                st.open(acc, &f);
            }
            if drive::export_format(&f.mime).is_some()
                && ui.add_sized([w, 30.0], egui::Button::new("Ouvrir avec une application du PC")).on_hover_text("Exporte en Office (docx, xlsx, pptx) puis ouvre").clicked()
            {
                st.open_local(acc, &f);
            }
        } else {
            if ui.add_sized([w, 30.0], egui::Button::new("Ouvrir")).on_hover_text("Télécharge puis ouvre avec l'application par défaut").clicked() {
                st.open(acc, &f);
            }
            if ui.add_sized([w, 30.0], egui::Button::new("Ouvrir dans le navigateur")).clicked()
                && let Some(u) = &f.web_link
            {
                let _ = webbrowser::open(u);
            }
        }
        if (!f.is_google() || drive::export_format(&f.mime).is_some()) && ui.add_sized([w, 30.0], egui::Button::new("Télécharger…")).clicked() {
            st.download(acc, &f, false);
        }
    }
    if let Some(u) = &f.web_link
        && ui.add_sized([w, 30.0], egui::Button::new("Copier le lien")).clicked()
    {
        ui.ctx().copy_text(u.clone());
        st.info("Lien copié");
    }
    if let Some(p) = f.parents.first()
        && !matches!(st.current(), Source::Folder(ref id) if id == p)
        && ui.add_sized([w, 30.0], egui::Button::new("Afficher l'emplacement")).clicked()
    {
        let p = p.clone();
        st.go(acc, Source::Folder(p));
        st.selected = Some(f.clone());
    }
}

fn toast(ui: &mut egui::Ui, st: &mut DriveState) {
    let Some((text, error, at)) = st.notice.clone() else { return };
    if at.elapsed() > Duration::from_secs(if error { 8 } else { 3 }) {
        st.notice = None;
        return;
    }
    egui::Area::new(egui::Id::unique("wt_drive_toast")).order(egui::Order::Foreground).anchor(Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0)).show(ui.ctx(), |ui| {
        egui::Frame::new()
            .fill(if error { Color32::from_rgb(0x5A, 0x22, 0x22) } else { theme::SURFACE_HI() })
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_max_width(380.0);
                ui.label(RichText::new(text).color(theme::TEXT()));
            });
    });
    ui.ctx().request_repaint_after(Duration::from_millis(300));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_never_split_a_character() {
        use chrono::TimeZone;
        for m in 1..=12 {
            let ms = Local.with_ymd_and_hms(2025, m, 15, 12, 0, 0).unwrap().timestamp_millis();
            assert!(!modified(ms).is_empty());
        }
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(size(512), "512 o");
        assert_eq!(size(2048), "2 Ko");
        assert_eq!(size(5 * 1048576), "5.0 Mo");
    }
}
