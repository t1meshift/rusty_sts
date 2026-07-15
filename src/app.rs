use crate::autosync;
use crate::config::{Config, API_URL};
use crate::detect;
use crate::startup;
use crate::sync::{self, SyncProgress, SyncResult};
use crate::tray::{self, TrayHandle, TrayMenuIds};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use tray_icon::menu::MenuId;

#[derive(Debug, Clone, PartialEq)]
enum AppState {
    Setup,
    Ready,
    Syncing,
}

pub struct StsApp {
    state: AppState,
    // Setup fields
    api_token: String,
    folder_path: String,
    detected_folders: Vec<String>,
    setup_error: Option<String>,
    // Config fields
    auto_sync: bool,
    start_with_windows: bool,
    // Ready state
    run_file_count: usize,
    new_run_count: usize,
    last_result: Option<SyncResult>,
    // Syncing state
    progress_rx: Option<mpsc::Receiver<SyncProgress>>,
    result_rx: Option<mpsc::Receiver<SyncResult>>,
    current_progress: Option<SyncProgress>,
    // Tray — must stay alive for the icon to remain visible
    _tray: tray::PlatformTray,
    tray_menu_ids: TrayMenuIds,
    tray_menu_rx: mpsc::Receiver<MenuId>,
    window_visible: bool,
    // Auto-sync
    sync_in_progress: Arc<AtomicBool>,
    auto_sync_enabled: Arc<AtomicBool>,
    autosync_started: bool,
    minimized_on_start: bool,
    _egui_ctx: egui::Context,
}

impl StsApp {
    pub fn new(tray_handle: TrayHandle, start_visible: bool, egui_ctx: egui::Context) -> Self {
        tray::set_egui_ctx(egui_ctx.clone());
        let _tray = tray_handle._platform;
        let tray_menu_ids = tray_handle.ids;
        let tray_menu_rx = tray_handle.menu_rx;
        let _tray_click_rx = tray_handle.click_rx;
        let detected = detect::detect_save_folders();
        let detected_folders: Vec<String> = detected
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let auto_folder = if detected_folders.len() == 1 {
            detected_folders[0].clone()
        } else {
            String::new()
        };

        let sync_in_progress = Arc::new(AtomicBool::new(false));

        match Config::load() {
            Some(config) => {
                let count = detect::count_run_files(&config.folder_path);
                let synced = Config::load_synced_runs();
                let new_count = detect::count_new_run_files(&config.folder_path, &synced);
                let auto_sync_enabled = Arc::new(AtomicBool::new(config.auto_sync));

                // Start autosync immediately — don't wait for update() which may not
                // run reliably when the window is hidden via SW_HIDE.
                autosync::start_polling(
                    config.folder_path.clone(),
                    config.api_token.clone(),
                    Arc::clone(&sync_in_progress),
                    Arc::clone(&auto_sync_enabled),
                );

                Self {
                    state: AppState::Ready,
                    api_token: config.api_token,
                    folder_path: config.folder_path,
                    detected_folders,
                    setup_error: None,
                    auto_sync: config.auto_sync,
                    start_with_windows: config.start_with_windows,
                    run_file_count: count,
                    new_run_count: new_count,
                    last_result: None,
                    progress_rx: None,
                    result_rx: None,
                    current_progress: None,
                    _tray,
                    tray_menu_ids,
                    tray_menu_rx,
                    window_visible: start_visible,
                    sync_in_progress,
                    auto_sync_enabled,
                    autosync_started: true,
                    minimized_on_start: false,
                    _egui_ctx: egui_ctx.clone(),
                }
            }
            None => {
                let auto_sync_enabled = Arc::new(AtomicBool::new(true));
                Self {
                    state: AppState::Setup,
                    api_token: String::new(),
                    folder_path: auto_folder,
                    detected_folders,
                    setup_error: None,
                    auto_sync: true,
                    start_with_windows: false,
                    run_file_count: 0,
                    new_run_count: 0,
                    last_result: None,
                    progress_rx: None,
                    result_rx: None,
                    current_progress: None,
                    _tray,
                    tray_menu_ids,
                    tray_menu_rx,
                    window_visible: start_visible,
                    sync_in_progress,
                    auto_sync_enabled,
                    autosync_started: false,
                    minimized_on_start: false,
                    _egui_ctx: egui_ctx.clone(),
                }
            }
        }
    }

    fn refresh_file_counts(&mut self) {
        self.run_file_count = detect::count_run_files(&self.folder_path);
        let synced = Config::load_synced_runs();
        self.new_run_count = detect::count_new_run_files(&self.folder_path, &synced);
    }

    fn save_config(&self) -> Result<(), String> {
        let config = Config {
            api_token: self.api_token.clone(),
            folder_path: self.folder_path.clone(),
            auto_sync: self.auto_sync,
            start_with_windows: self.start_with_windows,
        };
        config.validate()?;
        config.save()
    }

    fn start_sync(&mut self) {
        self.sync_in_progress.store(true, Ordering::Relaxed);

        let (progress_tx, progress_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        let api_url = API_URL.to_string();
        let api_token = self.api_token.clone();
        let folder_path = self.folder_path.clone();

        let sync_flag = Arc::clone(&self.sync_in_progress);
        std::thread::spawn(move || {
            let result = sync::run_sync(api_url, api_token, folder_path, progress_tx);
            let _ = result_tx.send(result);
            // Clear flag here too — if the window is hidden, render_syncing won't run
            sync_flag.store(false, Ordering::Relaxed);
        });

        self.progress_rx = Some(progress_rx);
        self.result_rx = Some(result_rx);
        self.current_progress = None;
        self.state = AppState::Syncing;
    }

    fn start_autosync_polling(&mut self) {
        if self.autosync_started {
            return;
        }

        autosync::start_polling(
            self.folder_path.clone(),
            self.api_token.clone(),
            Arc::clone(&self.sync_in_progress),
            Arc::clone(&self.auto_sync_enabled),
        );

        self.autosync_started = true;
    }

    fn handle_tray_events(&mut self) {
        // Open and Quit are handled directly in tray.rs callbacks.
        // Only Sync Now comes through the channel.
        while let Ok(id) = self.tray_menu_rx.try_recv() {
            if id == self.tray_menu_ids.sync_now {
                self.refresh_file_counts();
                if self.state == AppState::Ready && self.new_run_count > 0 {
                    self.start_sync();
                }
            }
        }
    }

    fn handle_close_requested(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            // Hide to tray instead of exiting
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window();
            self.window_visible = false;
        }
    }

    fn render_setup(&mut self, ui: &mut egui::Ui) {
        ui.heading("Setup");
        ui.add_space(8.0);

        ui.label("API Token (from Settings page on the website):");
        ui.add(egui::TextEdit::singleline(&mut self.api_token).desired_width(f32::INFINITY));
        ui.add_space(8.0);

        ui.label("Save Folder:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.folder_path)
                    .desired_width(ui.available_width() - 70.0),
            );
            if ui.button("Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.folder_path = path.to_string_lossy().to_string();
                }
            }
        });

        if !self.detected_folders.is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Detected folders:")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            let folders = self.detected_folders.clone();
            for folder in &folders {
                if ui
                    .small_button(truncate_path(folder, 60))
                    .on_hover_text(folder)
                    .clicked()
                {
                    self.folder_path = folder.clone();
                }
            }
        }

        ui.add_space(12.0);

        if let Some(err) = &self.setup_error {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err.as_str());
            ui.add_space(4.0);
        }

        if ui.button("Save & Continue").clicked() {
            match self.save_config() {
                Ok(()) => {
                    self.setup_error = None;
                    self.refresh_file_counts();
                    self.state = AppState::Ready;
                }
                Err(e) => {
                    self.setup_error = Some(e);
                }
            }
        }
    }

    fn render_ready(&mut self, ui: &mut egui::Ui) {
        ui.heading("Ready to Sync");
        ui.add_space(8.0);

        ui.label(format!("Folder: {}", self.folder_path));
        ui.label(format!(
            "{} .run files found ({} new)",
            self.run_file_count, self.new_run_count
        ));

        ui.add_space(12.0);

        ui.horizontal(|ui| {
            let sync_enabled = self.new_run_count > 0;
            if ui
                .add_enabled(
                    sync_enabled,
                    egui::Button::new(if self.new_run_count > 0 {
                        format!("Sync {} new runs", self.new_run_count)
                    } else {
                        "All synced".to_string()
                    }),
                )
                .clicked()
            {
                self.start_sync();
            }
            if ui.button("Settings").clicked() {
                self.state = AppState::Setup;
            }
        });

        // Settings toggles
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        let prev_auto_sync = self.auto_sync;
        ui.checkbox(&mut self.auto_sync, "Auto-sync new runs");
        if self.auto_sync != prev_auto_sync {
            let _ = self.save_config();
            // Update the shared flag so the polling thread respects the toggle
            self.auto_sync_enabled
                .store(self.auto_sync, Ordering::Relaxed);
            if self.auto_sync && !self.autosync_started {
                self.start_autosync_polling();
            }
        }

        let prev_start = self.start_with_windows;
        let autostart_label = if cfg!(windows) {
            "Start with Windows"
        } else {
            "Start at login"
        };
        ui.checkbox(&mut self.start_with_windows, autostart_label);
        if self.start_with_windows != prev_start {
            let _ = self.save_config();
            if self.start_with_windows {
                let _ = startup::enable_autostart();
            } else {
                let _ = startup::disable_autostart();
            }
        }

        if let Some(result) = &self.last_result {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Last Sync Result").strong());
            ui.label(format!("Imported: {}", result.imported));
            ui.label(format!("Skipped: {}", result.skipped));
            if !result.errors.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 100, 100),
                    format!("Errors: {}", result.errors.len()),
                );
                for err in &result.errors {
                    ui.label(
                        egui::RichText::new(format!("  - {err}"))
                            .small()
                            .color(egui::Color32::from_rgb(255, 150, 150)),
                    );
                }
            }
        }
    }

    fn render_syncing(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.heading("Syncing...");
        ui.add_space(8.0);

        if let Some(rx) = &self.progress_rx {
            while let Ok(progress) = rx.try_recv() {
                self.current_progress = Some(progress);
            }
        }

        let mut completed_result = None;
        if let Some(rx) = &self.result_rx {
            if let Ok(result) = rx.try_recv() {
                completed_result = Some(result);
            }
        }

        if let Some(result) = completed_result {
            self.sync_in_progress.store(false, Ordering::Relaxed);

            self.last_result = Some(result);
            self.progress_rx = None;
            self.result_rx = None;
            self.current_progress = None;
            self.refresh_file_counts();
            self.state = AppState::Ready;
            return;
        }

        if let Some(progress) = &self.current_progress {
            ui.label(&progress.phase);
            if progress.total > 0 {
                let fraction = progress.current as f32 / progress.total as f32;
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .text(format!("{}/{} files", progress.current, progress.total)),
                );
            }
        } else {
            ui.label("Starting...");
            ui.spinner();
        }

        ctx.request_repaint();
    }
}

impl eframe::App for StsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Start auto-sync polling if ready and not yet started.
        // The thread checks the auto_sync_enabled flag internally, so start it
        // regardless of the current auto_sync setting — it will respect toggling.
        if self.state != AppState::Setup && !self.autosync_started {
            self.start_autosync_polling();
        }

        // Minimize on first frame if starting minimized
        if !self.window_visible && !self.minimized_on_start {
            if cfg!(windows) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            } else {
                // Hide to tray — Linux has no Win32-style hidden-minimize
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            self.minimized_on_start = true;
        }

        // Keep the event loop alive even when minimized,
        // so we can process tray events and auto-sync requests.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        self.handle_tray_events();
        self.handle_close_requested(ctx);

        egui::CentralPanel::default().show(ctx, |ui| match self.state.clone() {
            AppState::Setup => self.render_setup(ui),
            AppState::Ready => self.render_ready(ui),
            AppState::Syncing => self.render_syncing(ctx, ui),
        });
    }
}

fn truncate_path(path: &str, max_len: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max_len {
        path.to_string()
    } else {
        let suffix: String = chars[chars.len() - max_len + 3..].iter().collect();
        format!("...{suffix}")
    }
}
