use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::SystemTime;

const BATCH_SIZE: usize = 10;

/// reqwest's top-level Display is vague ("error sending request for url");
/// the actual cause (DNS, TLS, connection refused, timeout) is in the
/// source chain — include it.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut source = e.source();
    while let Some(src) = source {
        msg.push_str(": ");
        msg.push_str(&src.to_string());
        source = src.source();
    }
    msg
}

/// Keep response bodies in logs readable — they can be large HTML error pages.
fn truncate_for_log(body: &str) -> &str {
    let end = body
        .char_indices()
        .nth(2000)
        .map(|(i, _)| i)
        .unwrap_or(body.len());
    &body[..end]
}

#[derive(Debug, Clone)]
pub struct SyncProgress {
    pub current: usize,
    pub total: usize,
    pub phase: String,
}

#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SyncRequestFile {
    filename: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct SyncRequest {
    files: Vec<SyncRequestFile>,
}

#[derive(Debug, Deserialize)]
struct SyncResponse {
    #[serde(default)]
    imported: usize,
    #[serde(default)]
    skipped: usize,
    #[serde(default)]
    errors: Vec<String>,
}

pub fn run_sync(
    api_url: String,
    api_token: String,
    folder_path: String,
    progress_tx: mpsc::Sender<SyncProgress>,
) -> SyncResult {
    let dir = PathBuf::from(&folder_path);
    let all_run_files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "run"))
            .collect(),
        Err(e) => {
            return SyncResult {
                errors: vec![format!("Failed to read directory: {e}")],
                ..Default::default()
            };
        }
    };

    // Filter out already-synced runs
    let mut synced = Config::load_synced_runs();
    let run_files: Vec<PathBuf> = all_run_files
        .into_iter()
        .filter(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            !synced.contains(&name)
        })
        .collect();

    let total = run_files.len();
    if total == 0 {
        let _ = progress_tx.send(SyncProgress {
            current: 0,
            total: 0,
            phase: "All runs already synced".to_string(),
        });
        return SyncResult::default();
    }

    let sync_url = format!("{}/api/runs/sync", api_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::new();
    let mut result = SyncResult::default();
    let mut newly_synced: Vec<String> = Vec::new();
    let batches: Vec<&[PathBuf]> = run_files.chunks(BATCH_SIZE).collect();
    let mut files_processed = 0;

    for (batch_idx, batch) in batches.iter().enumerate() {
        let _ = progress_tx.send(SyncProgress {
            current: files_processed,
            total,
            phase: format!("Uploading batch {}/{}", batch_idx + 1, batches.len()),
        });

        let mut request_files = Vec::new();

        for file_path in *batch {
            match std::fs::read_to_string(file_path) {
                Ok(content) => {
                    let filename = file_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    request_files.push(SyncRequestFile { filename, content });
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to read {}: {e}", file_path.display()));
                }
            }
        }

        if request_files.is_empty() {
            files_processed += batch.len();
            continue;
        }

        let body = SyncRequest {
            files: request_files,
        };

        match client
            .post(&sync_url)
            .header("Authorization", format!("Bearer {}", api_token))
            .json(&body)
            .send()
        {
            Ok(response) => {
                let status = response.status();
                let body_text = response.text().unwrap_or_default();
                if !status.is_success() {
                    eprintln!(
                        "[sync] POST {sync_url} -> {status}: {}",
                        truncate_for_log(&body_text)
                    );
                }
                if status.is_success() {
                    // Track filenames from this batch as synced
                    let batch_filenames: Vec<String> = batch
                        .iter()
                        .map(|p| {
                            p.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        })
                        .collect();
                    match serde_json::from_str::<SyncResponse>(&body_text) {
                        Ok(resp) => {
                            result.imported += resp.imported;
                            result.skipped += resp.skipped;
                            result.errors.extend(resp.errors);
                            newly_synced.extend(batch_filenames);
                        }
                        Err(e) => {
                            result.errors.push(format!("Failed to parse response: {e}"));
                        }
                    }
                } else if status.as_u16() == 401 {
                    result
                        .errors
                        .push("Token rejected (401 Unauthorized)".to_string());
                    break;
                } else if status.as_u16() == 413 {
                    result
                        .errors
                        .push("Batch too large (413 Payload Too Large)".to_string());
                } else {
                    result
                        .errors
                        .push(format!("Server error ({status}): {body_text}"));
                }
            }
            Err(e) => {
                let msg = format!("Network error: {}", error_chain(&e));
                eprintln!("[sync] POST {sync_url} failed: {msg}");
                result.errors.push(msg);
                break;
            }
        }

        files_processed += batch.len();
    }

    // Save newly synced filenames
    if !newly_synced.is_empty() {
        for name in &newly_synced {
            synced.insert(name.clone());
        }
        let _ = Config::save_synced_runs(&synced);
    }

    let _ = progress_tx.send(SyncProgress {
        current: total,
        total,
        phase: "Done".to_string(),
    });

    result
}

/// Derives the current_run.save path from the history folder path.
/// history folder: .../saves/history → save file: .../saves/current_run.save
pub fn current_run_save_path(history_folder: &str) -> Option<PathBuf> {
    let history = PathBuf::from(history_folder);
    let saves_dir = history.parent()?; // .../saves/
    let save_path = saves_dir.join("current_run.save");
    if save_path.exists() {
        Some(save_path)
    } else {
        None
    }
}

/// Returns the last modified time of the current_run.save file.
pub fn current_run_modified_time(history_folder: &str) -> Option<SystemTime> {
    let path = current_run_save_path(history_folder)?;
    std::fs::metadata(&path).ok()?.modified().ok()
}

/// Uploads the current_run.save content to PUT /api/companion/active-run.
/// Returns Ok(true) if uploaded, Ok(false) if file doesn't exist, Err on failure.
pub fn sync_active_run(
    api_url: &str,
    api_token: &str,
    history_folder: &str,
) -> Result<bool, String> {
    let path = match current_run_save_path(history_folder) {
        Some(p) => p,
        None => return Ok(false),
    };

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read current_run.save: {e}"))?;

    // Validate it's valid JSON with expected fields
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in current_run.save: {e}"))?;

    if json.get("players").is_none() || json.get("map_point_history").is_none() {
        return Ok(false); // Not a valid save file (maybe empty/corrupt)
    }

    let url = format!("{}/api/companion/active-run", api_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::new();

    let response = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", api_token))
        .header("Content-Type", "application/json")
        .body(content)
        .send()
        .map_err(|e| format!("Network error syncing active run: {}", error_chain(&e)))?;

    // Failures propagate as Err — the caller logs them.
    let status = response.status();
    if status.is_success() {
        Ok(true)
    } else {
        let body = response.text().unwrap_or_default();
        Err(format!(
            "Failed to sync active run ({status}): {}",
            truncate_for_log(&body)
        ))
    }
}
