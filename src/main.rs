//! WorkTable — v0
//! Organisation de travail : tableau de bord, calendrier, mail (Gmail), notes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod emoji_img;
mod google;
mod links;
mod liquid;
mod notes_store;
mod notify;
mod paths;
mod prefs;
mod search;
mod sidebar;
mod tabs;
mod tasks;
mod theme;
mod widgets;

/// En cas de plantage, la cause est écrite dans `data\config\crash.log` :
/// l'application n'a pas de console sous Windows.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = Some(crate::paths::config()) {
            let _ = std::fs::create_dir_all(&dir);
            let text = format!(
                "{} — {}\n{}\n\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                info,
                std::backtrace::Backtrace::force_capture()
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

fn main() -> eframe::Result<()> {
    install_crash_log();
    // Toutes les données dans le dossier `data\` (transportable) ; reprise de l'ancien emplacement.
    paths::migrate();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("WorkTable")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([880.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "WorkTable",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::WorkTableApp::new(&cc.egui_ctx)))
        }),
    )
}
