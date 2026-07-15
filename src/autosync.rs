use crate::config::{Config, API_URL};
use crate::detect;
use crate::notification;
use crate::sync;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const ACTIVE_RUN_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Starts the auto-sync polling loop in a background thread.
/// Performs the sync directly — does not depend on the eframe event loop.
pub fn start_polling(
    folder_path: String,
    api_token: String,
    sync_in_progress: Arc<AtomicBool>,
    auto_sync_enabled: Arc<AtomicBool>,
) {
    let folder_for_runs = folder_path.clone();
    let token_for_runs = api_token.clone();
    let sync_flag_for_runs = sync_in_progress.clone();
    let auto_flag_for_runs = auto_sync_enabled.clone();

    // Run file sync thread (every 60s)
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL_INTERVAL);

        if !auto_flag_for_runs.load(Ordering::Relaxed) {
            continue;
        }

        if sync_flag_for_runs.load(Ordering::Relaxed) {
            continue;
        }

        let synced = Config::load_synced_runs();
        let new_count = detect::count_new_run_files(&folder_for_runs, &synced);

        if new_count > 0 {
            sync_flag_for_runs.store(true, Ordering::Relaxed);

            let (progress_tx, _progress_rx) = std::sync::mpsc::channel();
            let result = sync::run_sync(
                API_URL.to_string(),
                token_for_runs.clone(),
                folder_for_runs.clone(),
                progress_tx,
            );

            if result.imported > 0 {
                notification::notify_sync_complete(result.imported);
            }

            sync_flag_for_runs.store(false, Ordering::Relaxed);
        }
    });

    // Active run sync thread (every 10s, checks mtime)
    std::thread::spawn(move || {
        let mut last_mtime: Option<SystemTime> = None;

        eprintln!("[active-run] Thread started, watching folder: {folder_path}");

        // Check if save file path resolves
        match sync::current_run_save_path(&folder_path) {
            Some(p) => eprintln!("[active-run] Save file found at: {}", p.display()),
            None => {
                let expected = std::path::Path::new(&folder_path)
                    .parent()
                    .map(|p| p.join("current_run.save").display().to_string())
                    .unwrap_or_else(|| format!("<parent of {folder_path}>/current_run.save"));
                eprintln!(
                    "[active-run] No {expected} yet — it appears while a run is in progress"
                );
            }
        }

        loop {
            std::thread::sleep(ACTIVE_RUN_POLL_INTERVAL);

            if !auto_sync_enabled.load(Ordering::Relaxed) {
                continue;
            }

            let current_mtime = sync::current_run_modified_time(&folder_path);

            // Only upload if the file has changed since last check
            let changed = match (current_mtime, last_mtime) {
                (Some(current), Some(last)) => current != last,
                (Some(_), None) => true,  // File appeared
                (None, Some(_)) => false, // File disappeared, do nothing
                (None, None) => false,
            };

            if changed {
                match sync::sync_active_run(API_URL, &api_token, &folder_path) {
                    Ok(true) => {
                        eprintln!("[active-run] Synced current_run.save");
                        last_mtime = current_mtime;
                    }
                    Ok(false) => {
                        eprintln!("[active-run] No valid save file found");
                        last_mtime = None;
                    }
                    Err(e) => {
                        eprintln!("[active-run] Error: {e}");
                        // Don't update mtime, will retry next cycle
                    }
                }
            }
        }
    });
}
