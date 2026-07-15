#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod autosync;
mod config;
mod detect;
mod notification;
mod startup;
mod sync;
mod tray;

fn main() -> eframe::Result {
    let start_minimized = std::env::args().any(|a| a == "--minimized");

    // Load config once — used for visibility check and autostart refresh
    let loaded_config = config::Config::load();
    let has_config = loaded_config.is_some();
    let start_visible = !start_minimized || !has_config;

    // Keep the autostart entry pointing at the current exe path
    if let Some(cfg) = &loaded_config {
        if cfg.start_with_windows {
            startup::refresh_autostart_if_needed();
        }
    }
    drop(loaded_config);

    // Create tray icon before the event loop.
    // tray_handle must stay alive (not dropped) for the icon to remain visible.
    let tray_handle = tray::create_tray().expect("Failed to create tray icon");

    // Load the icon for the window titlebar and taskbar
    let icon_bytes = include_bytes!("../assets/icon.png");
    let icon_img = image::load_from_memory(icon_bytes)
        .expect("Failed to load icon")
        .into_rgba8();
    let (w, h) = icon_img.dimensions();
    let window_icon = egui::IconData {
        rgba: icon_img.into_raw(),
        width: w,
        height: h,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([440.0, 380.0])
            .with_title("rusty-sts")
            .with_icon(std::sync::Arc::new(window_icon)),
        ..Default::default()
    };

    eframe::run_native(
        "rusty-sts",
        options,
        Box::new(move |cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = egui::Color32::from_rgb(14, 14, 18);
            visuals.panel_fill = egui::Color32::from_rgb(14, 14, 18);
            visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(24, 24, 30);
            visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(30, 30, 38);
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(40, 40, 50);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(50, 50, 62);
            visuals.selection.bg_fill = egui::Color32::from_rgb(59, 130, 246);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(app::StsApp::new(
                tray_handle,
                start_visible,
                cc.egui_ctx.clone(),
            )))
        }),
    )
}
