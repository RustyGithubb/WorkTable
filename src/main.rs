//! WorkTable — v0
//! Organisation de travail : tableau de bord, calendrier, mail (Gmail), notes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod emoji_img;
mod google;
mod lightbox;
mod links;
mod liquid;
mod notes_store;
mod notify;
mod paths;
mod prefs;
mod search;
mod sidebar;
mod sound;
mod tabs;
mod tasks;
mod theme;
mod titlebar;
mod widgets;

/// En cas de plantage, la cause est écrite dans `data\config\crash.log` :
/// l'application n'a pas de console sous Windows.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = Some(crate::paths::config()) {
            let _ = std::fs::create_dir_all(&dir);
            let text = format!("{} — {}\n{}\n\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), info, std::backtrace::Backtrace::force_capture());
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

fn options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("WorkTable")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([880.0, 560.0])
            // Tampon d'image avec transparence : indispensable pour que la fenêtre du bandeau de
            // notification soit transparente. La fenêtre principale reste opaque (fond peint).
            .with_transparent(true)
            // Pas de barre de titre Windows : l'app dessine la sienne (déplacement, boutons, bords).
            .with_decorations(false)
            // Icône de la fenêtre et de la barre des tâches.
            .with_icon(eframe::icon_data::from_png_bytes(sidebar::LOGO_PNG).unwrap_or_default()),
        renderer,
        ..Default::default()
    }
}

fn app_creator() -> eframe::AppCreator<'static> {
    Box::new(|cc| {
        // Identifiant Windows de la fenêtre principale (pour la remettre au premier plan).
        if let Ok(h) = raw_window_handle::HasWindowHandle::window_handle(cc)
            && let raw_window_handle::RawWindowHandle::Win32(w) = h.as_raw()
        {
            notify::set_root_window(w.hwnd.get());
        }
        theme::apply(&cc.egui_ctx);
        Ok(Box::new(app::WorkTableApp::new(&cc.egui_ctx)))
    })
}

fn main() -> eframe::Result<()> {
    install_crash_log();
    // Toutes les données dans le dossier `data\` (transportable) ; reprise de l'ancien emplacement.
    paths::migrate();
    // Windows : DirectX 12 avec une chaîne d'affichage « DirectComposition », la seule qui gère la
    // vraie transparence (bandeau de notification sans fond, avec son ombre). Si DirectX 12 n'est pas
    // disponible, repli sur OpenGL (le bandeau est alors découpé à sa forme, sans ombre).
    // `WORKTABLE_RENDERER=glow` ou `=wgpu` force un moteur.
    let forced = std::env::var("WORKTABLE_RENDERER").ok();
    let dx12 = cfg!(windows) && forced.is_none() && dx12_available();
    if dx12 {
        // SAFETY : appelé au démarrage, avant tout autre fil d'exécution.
        unsafe {
            if std::env::var_os("WGPU_BACKEND").is_none() {
                std::env::set_var("WGPU_BACKEND", "dx12");
            }
            if std::env::var_os("WGPU_DX12_PRESENTATION_SYSTEM").is_none() {
                std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "Visual");
            }
        }
    }
    let renderer = match forced.as_deref() {
        Some("glow") => eframe::Renderer::Glow,
        Some(_) => eframe::Renderer::Wgpu,
        None if cfg!(windows) && !dx12 => eframe::Renderer::Glow,
        None => eframe::Renderer::Wgpu,
    };
    // Sans DirectComposition, Windows n'affiche pas la transparence : bandeau découpé à sa forme.
    notify::set_clip_shape(cfg!(windows) && !dx12);
    eframe::run_native("WorkTable", options(renderer), app_creator())
}

/// Une carte graphique compatible DirectX 12 est-elle disponible ?
fn dx12_available() -> bool {
    use eframe::wgpu;
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(desc);
    // L'énumération est immédiate sur ordinateur : une seule interrogation suffit.
    let mut fut = std::pin::pin!(instance.enumerate_adapters(wgpu::Backends::DX12));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(list) => !list.is_empty(),
        std::task::Poll::Pending => false,
    }
}
