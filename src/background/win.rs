//! Partie Windows de l'arrière-plan : fenêtre cachée, raccourci global, instance unique.
//!
//! La fenêtre « cachée » reste en réalité affichée, hors de l'écran et sans bouton dans la barre
//! des tâches : Windows ne redessine pas une fenêtre masquée ou réduite, et l'app ne tournerait
//! plus (egui ne fait son travail qu'en dessinant).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use eframe::egui;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, GetCurrentThreadId, INFINITE, OpenEventW, SetEvent};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW, GetWindowPlacement, IsIconic, MSG,
    MsgWaitForMultipleObjects, PM_NOREMOVE, PM_REMOVE, PeekMessageW, PostThreadMessageW, QS_ALLINPUT, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_RESTORE, SW_SHOWMAXIMIZED,
    SW_SHOWMINIMIZED, SW_SHOWNORMAL, SWP_FRAMECHANGED, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, ShowWindow, WINDOWPLACEMENT, WM_APP, WM_HOTKEY, WPF_RESTORETOMAXIMIZED, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_MAXIMIZE,
    WS_MINIMIZE,
};

/// Fil d'écoute (pour lui signaler un nouveau raccourci).
static THREAD: AtomicU32 = AtomicU32::new(0);
/// Raccourci voulu (modificateurs, touche).
static WANTED: Mutex<Option<(u32, u32)>> = Mutex::new(None);
/// 0 : rien d'enregistré, 1 : actif, 2 : refusé (déjà pris par une autre application).
static STATUS: AtomicU8 = AtomicU8::new(0);
static HIDDEN: AtomicBool = AtomicBool::new(false);
/// Placement et style étendu de la fenêtre avant qu'elle soit cachée.
static SAVED: Mutex<Option<(WINDOWPLACEMENT, isize)>> = Mutex::new(None);
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

const HOTKEY_ID: i32 = 1;
/// Message « raccourci modifié » envoyé au fil d'écoute.
const WM_RELOAD: u32 = WM_APP + 1;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn root() -> Option<HWND> {
    let h = crate::notify::root_window();
    (h != 0).then_some(h as HWND)
}

fn repaint() {
    if let Some(ctx) = CTX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
}

/// Instance unique, par mode de lancement (installée, portable, développement) : `false` si une
/// autre tourne déjà ; elle est alors rouverte. `replace` : on lui demande plutôt de quitter et on
/// prend sa place (nouvelle compilation en développement) ; après une mise à jour, l'ancienne
/// version se ferme d'elle-même. On attend alors sa fermeture (10 s au plus).
pub fn single_instance(mode: &str, after_update: bool, replace: bool) -> bool {
    let name = wide(&format!("Local\\WorkTable.Instance.{mode}"));
    let wait = after_update || replace;
    for k in 0..100 {
        // SAFETY : nom terminé par un zéro, valide le temps de l'appel ; le verrou est gardé
        // ouvert jusqu'à la fin du processus.
        unsafe {
            let m = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if m.is_null() {
                return true;
            }
            if GetLastError() != ERROR_ALREADY_EXISTS {
                return true;
            }
            CloseHandle(m);
        }
        if !wait {
            break;
        }
        if k == 0 && replace && !after_update {
            signal(&format!("Local\\WorkTable.Quit.{mode}"));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Une autre instance tourne : on lui demande de se montrer (et l'autorise à passer devant).
    // SAFETY : autorisation sans pointeur.
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
    signal(&format!("Local\\WorkTable.Show.{mode}"));
    false
}

/// Déclenche un événement nommé d'une autre instance (s'il existe).
fn signal(name: &str) {
    let name = wide(name);
    // SAFETY : nom valide le temps de l'appel ; poignée fermée aussitôt.
    unsafe {
        let e = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if !e.is_null() {
            SetEvent(e);
            CloseHandle(e);
        }
    }
}

/// Fil d'écoute : raccourci global (messages `WM_HOTKEY`) et demandes de réouverture (événement
/// déclenché par une autre instance).
pub fn start(ctx: egui::Context, hotkey: Option<(u32, u32)>) {
    black_background();
    *CTX.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = hotkey;
    let mode = format!("{:?}", crate::paths::mode());
    let _ = std::thread::Builder::new().name("wt-hotkey".into()).spawn(move || {
        let show_name = wide(&format!("Local\\WorkTable.Show.{mode}"));
        let quit_name = wide(&format!("Local\\WorkTable.Quit.{mode}"));
        // SAFETY : appels Win32 sur des structures locales ; les événements vivent jusqu'à la fin
        // du processus.
        unsafe {
            let show_ev: HANDLE = CreateEventW(std::ptr::null(), 0, 0, show_name.as_ptr());
            let quit_ev: HANDLE = CreateEventW(std::ptr::null(), 0, 0, quit_name.as_ptr());
            let mut msg: MSG = std::mem::zeroed();
            // Crée la file de messages du fil avant d'annoncer son identifiant.
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
            register();
            loop {
                // Réouverture demandée par une autre instance, ou fermeture (une nouvelle version
                // prend la place).
                let handles: Vec<HANDLE> = [show_ev, quit_ev].into_iter().filter(|h| !h.is_null()).collect();
                let r = MsgWaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE, QS_ALLINPUT) as usize;
                match handles.get(r) {
                    Some(h) if *h == show_ev => {
                        show();
                    }
                    Some(_) => {
                        if let Some(ctx) = CTX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                            super::quit(ctx);
                            ctx.request_repaint();
                        }
                    }
                    None => {}
                }
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    match msg.message {
                        WM_HOTKEY => toggle(),
                        WM_RELOAD => register(),
                        _ => {}
                    }
                }
            }
        }
    });
}

/// (Ré)enregistre le raccourci voulu sur le fil d'écoute.
fn register() {
    let wanted = *WANTED.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY : raccourci attaché au fil courant (pas de fenêtre).
    unsafe {
        UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
        let status = match wanted {
            Some((mods, vk)) if RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, mods | MOD_NOREPEAT, vk) != 0 => 1,
            Some(_) => 2,
            None => 0,
        };
        STATUS.store(status, Ordering::Relaxed);
    }
    repaint();
}

/// Nouveau raccourci (`None` : désactivé, par exemple pendant qu'on en tape un autre).
pub fn set_hotkey(hotkey: Option<(u32, u32)>) {
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = hotkey;
    let t = THREAD.load(Ordering::Relaxed);
    if t != 0 {
        // SAFETY : simple message sans données vers le fil d'écoute.
        unsafe {
            PostThreadMessageW(t, WM_RELOAD, 0, 0);
        }
    }
}

pub fn hotkey_ok() -> Option<bool> {
    match STATUS.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Raccourci : affiche l'app, ou la cache si elle est déjà devant.
fn toggle() {
    let front = root().is_some_and(|h| unsafe { GetForegroundWindow() } == h);
    if HIDDEN.load(Ordering::Relaxed) || !front || super::closing() {
        show();
    } else {
        // Animation de fermeture (fil de l'interface), qui cache ensuite la fenêtre.
        super::request_close();
        repaint();
    }
}

/// Fond de la classe de fenêtre en noir (transparent sous DirectComposition) au lieu du blanc :
/// Windows en repeint la fenêtre quand elle réapparaît ou passe au premier plan, et ce blanc
/// transparaissait sous l'interface à moitié opaque pendant l'animation d'ouverture.
fn black_background() {
    use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GCLP_HBRBACKGROUND, SetClassLongPtrW};
    let Some(hwnd) = root() else {
        return;
    };
    // SAFETY : fenêtre de ce processus ; pinceau système (jamais libéré).
    unsafe {
        SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, GetStockObject(BLACK_BRUSH) as isize);
    }
}

/// Fond Windows de la fenêtre (sous l'image DirectX) rempli de noir transparent : sans cela il
/// apparaît en blanc autour de l'interface réduite pendant l'animation (voir le bandeau de
/// notification, même correctif).
pub fn clear_backdrop() {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, FillRect, GetDC, GetStockObject, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let Some(hwnd) = root() else {
        return;
    };
    // SAFETY : fenêtre principale de ce processus ; le contexte de dessin est rendu aussitôt.
    unsafe {
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetClientRect(hwnd, &mut rc) == 0 {
            return;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return;
        }
        FillRect(dc, &rc, GetStockObject(BLACK_BRUSH));
        ReleaseDC(hwnd, dc);
    }
}

pub fn can_hide() -> bool {
    root().is_some()
}

pub fn is_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// Cache la fenêtre : hors de l'écran, sans bouton dans la barre des tâches ni dans Alt+Tab.
pub fn hide() -> bool {
    let Some(hwnd) = root() else {
        return false;
    };
    if HIDDEN.swap(true, Ordering::Relaxed) {
        return true;
    }
    // SAFETY : fenêtre principale de ce processus ; structures locales.
    unsafe {
        let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
        wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(hwnd, &mut wp);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        ShowWindow(hwnd, SW_HIDE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize));
        // Ni agrandie ni réduite pendant qu'elle est cachée (rétabli à la réouverture).
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !((WS_MAXIMIZE | WS_MINIMIZE) as isize));
        let r = wp.rcNormalPosition;
        let (w, h) = ((r.right - r.left).max(200), (r.bottom - r.top).max(200));
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN) - w - 400;
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW | SWP_FRAMECHANGED);
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some((wp, ex));
    }
    true
}

/// Rouvre la fenêtre là où elle était (agrandie si elle l'était), au premier plan.
pub fn show() -> bool {
    let Some(hwnd) = root() else {
        return false;
    };
    let saved = SAVED.lock().unwrap_or_else(|e| e.into_inner()).take();
    // SAFETY : fenêtre principale de ce processus ; structures locales.
    unsafe {
        let was_hidden = HIDDEN.swap(false, Ordering::Relaxed);
        if was_hidden || super::closing() {
            super::opened();
        }
        if was_hidden && let Some((mut wp, ex)) = saved {
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
            if wp.showCmd == SW_SHOWMINIMIZED as u32 {
                wp.showCmd = if wp.flags & WPF_RESTORETOMAXIMIZED != 0 { SW_SHOWMAXIMIZED as u32 } else { SW_SHOWNORMAL as u32 };
            } else if wp.showCmd != SW_SHOWMAXIMIZED as u32 {
                wp.showCmd = SW_SHOWNORMAL as u32;
            }
            SetWindowPlacement(hwnd, &wp);
            // Fond transparent dès la réapparition (pas de flash blanc avant la première image).
            clear_backdrop();
        } else if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    repaint();
    true
}
