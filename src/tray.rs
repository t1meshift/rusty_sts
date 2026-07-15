use std::sync::mpsc;
use std::sync::OnceLock;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

// Set by the app once eframe is running; used to wake the event loop and
// show/hide the window from tray callbacks (which run off the main thread).
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn set_egui_ctx(ctx: egui::Context) {
    let _ = EGUI_CTX.set(ctx);
}

pub struct TrayMenuIds {
    pub open: MenuId,
    pub sync_now: MenuId,
    pub quit: MenuId,
}

/// Keeps the tray icon alive. On Windows the icon lives here; on Linux it
/// lives on the dedicated GTK thread (it cannot be sent across threads).
pub struct PlatformTray {
    #[cfg(windows)]
    _icon: tray_icon::TrayIcon,
}

pub struct TrayHandle {
    pub _platform: PlatformTray,
    pub ids: TrayMenuIds,
    pub menu_rx: mpsc::Receiver<MenuId>,
    pub click_rx: mpsc::Receiver<TrayIconEvent>,
}

#[cfg(windows)]
fn find_window() -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe { FindWindowW(None, w!("rusty-sts")).ok() }
}

/// Restore + focus the window.
/// On Windows this uses the Win32 API directly, which works regardless of
/// the eframe event loop state.
#[cfg(windows)]
fn show_window() {
    use windows::Win32::UI::WindowsAndMessaging::*;
    if let Some(hwnd) = find_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
fn show_window() {
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

/// Hide the window (to tray). On Windows this removes it from the taskbar
/// entirely via SW_HIDE; elsewhere it goes through the viewport.
#[cfg(windows)]
pub fn hide_window() {
    use windows::Win32::UI::WindowsAndMessaging::*;
    if let Some(hwnd) = find_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_window() {
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        ctx.request_repaint();
    }
}

fn build_tray_parts() -> Result<(tray_icon::TrayIcon, TrayMenuIds), String> {
    let icon_bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(icon_bytes)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = img.dimensions();
    let icon = Icon::from_rgba(img.into_raw(), width, height).map_err(|e| e.to_string())?;

    let menu = Menu::new();
    let open_item = MenuItem::new("Open", true, None);
    let sync_item = MenuItem::new("Sync Now", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    let ids = TrayMenuIds {
        open: open_item.id().clone(),
        sync_now: sync_item.id().clone(),
        quit: quit_item.id().clone(),
    };

    menu.append(&open_item).map_err(|e| e.to_string())?;
    menu.append(&sync_item).map_err(|e| e.to_string())?;
    menu.append(&quit_item).map_err(|e| e.to_string())?;

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("rusty-sts")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| e.to_string())?;

    Ok((tray_icon, ids))
}

/// Handle Open and Quit directly in callbacks, bypassing eframe's event loop
/// which doesn't run reliably when the window is hidden. Other events (like
/// Sync Now) are forwarded to channels polled by the app.
fn install_handlers(
    ids: &TrayMenuIds,
) -> (mpsc::Receiver<MenuId>, mpsc::Receiver<TrayIconEvent>) {
    let open_id = ids.open.clone();
    let quit_id = ids.quit.clone();
    let (menu_tx, menu_rx) = mpsc::channel();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == open_id {
            show_window();
        } else if event.id == quit_id {
            std::process::exit(0);
        } else {
            let _ = menu_tx.send(event.id);
            // Wake the eframe loop so the event is processed promptly
            if let Some(ctx) = EGUI_CTX.get() {
                ctx.request_repaint();
            }
        }
    }));

    let (click_tx, click_rx) = mpsc::channel();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = event
        {
            show_window();
        } else {
            let _ = click_tx.send(event);
        }
    }));

    (menu_rx, click_rx)
}

#[cfg(windows)]
pub fn create_tray() -> Result<TrayHandle, Box<dyn std::error::Error>> {
    let (tray_icon, ids) = build_tray_parts()?;
    let (menu_rx, click_rx) = install_handlers(&ids);

    Ok(TrayHandle {
        _platform: PlatformTray { _icon: tray_icon },
        ids,
        menu_rx,
        click_rx,
    })
}

/// On Linux the tray icon must be created and driven by a GTK event loop,
/// which eframe/winit does not provide — so it runs on its own thread.
#[cfg(target_os = "linux")]
pub fn create_tray() -> Result<TrayHandle, Box<dyn std::error::Error>> {
    let (ids_tx, ids_rx) = mpsc::channel::<Result<TrayMenuIds, String>>();

    std::thread::spawn(move || {
        if let Err(e) = gtk::init() {
            let _ = ids_tx.send(Err(format!("Failed to init GTK: {e}")));
            return;
        }
        match build_tray_parts() {
            Ok((tray_icon, ids)) => {
                let _ = ids_tx.send(Ok(ids));
                gtk::main();
                drop(tray_icon);
            }
            Err(e) => {
                let _ = ids_tx.send(Err(e));
            }
        }
    });

    let ids = ids_rx.recv().map_err(|e| e.to_string())??;
    let (menu_rx, click_rx) = install_handlers(&ids);

    Ok(TrayHandle {
        _platform: PlatformTray {},
        ids,
        menu_rx,
        click_rx,
    })
}
