use std::collections::HashSet;
use std::path::PathBuf;

/// Scan known locations for STS2 save history folders.
/// On Windows: %APPDATA%\SlayTheSpire2\steam\<steam_id>\profile1\saves\history\
pub fn detect_save_folders() -> Vec<PathBuf> {
    let mut results = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let base = PathBuf::from(appdata).join("SlayTheSpire2").join("steam");
            if base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&base) {
                    for entry in entries.flatten() {
                        let history = entry.path().join("profile1").join("saves").join("history");
                        if history.is_dir() {
                            results.push(history);
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // On macOS/Linux, check common save locations.
        // The Proton wine prefix (app id 2868840) covers anyone running the
        // Windows build of the game through Steam Play.
        const PROTON_SUFFIX: &str = "steamapps/compatdata/2868840/pfx/drive_c/users/steamuser/AppData/Roaming/SlayTheSpire2/steam";
        let mut candidates = Vec::new();
        // Native Linux build: ~/.local/share/SlayTheSpire2/steam/<steam_id>/
        if let Some(data) = dirs::data_dir() {
            candidates.push(data.join("SlayTheSpire2").join("steam"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.extend([
                home.join(".local/share/Steam").join(PROTON_SUFFIX),
                home.join(".steam/steam").join(PROTON_SUFFIX),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")
                    .join(PROTON_SUFFIX),
                home.join("snap/steam/common/.local/share/Steam")
                    .join(PROTON_SUFFIX),
                home.join("Library/Application Support/SlayTheSpire2/steam"),
            ]);
        }
        // Some candidates are symlinks to the same Steam library —
        // dedupe by canonical path.
        let mut seen = HashSet::new();
        for base in candidates {
            if base.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&base) {
                    for entry in entries.flatten() {
                        let history = entry.path().join("profile1").join("saves").join("history");
                        if history.is_dir() {
                            let canonical =
                                history.canonicalize().unwrap_or_else(|_| history.clone());
                            if seen.insert(canonical) {
                                results.push(history);
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

/// Count .run files not yet synced.
pub fn count_new_run_files(folder: &str, synced: &HashSet<String>) -> usize {
    let path = PathBuf::from(folder);
    if !path.is_dir() {
        return 0;
    }
    std::fs::read_dir(&path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let p = e.path();
                    p.extension().is_some_and(|ext| ext == "run")
                        && !synced.contains(
                            &p.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                        )
                })
                .count()
        })
        .unwrap_or(0)
}

/// Count .run files in a directory.
pub fn count_run_files(folder: &str) -> usize {
    let path = PathBuf::from(folder);
    if !path.is_dir() {
        return 0;
    }
    std::fs::read_dir(&path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "run"))
                .count()
        })
        .unwrap_or(0)
}
