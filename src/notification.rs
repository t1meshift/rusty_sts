fn sync_message(imported: usize) -> String {
    if imported == 1 {
        "1 new run synced".to_string()
    } else {
        format!("{imported} new runs synced")
    }
}

#[cfg(windows)]
pub fn notify_sync_complete(imported: usize) {
    use winrt_notification::{Duration, Toast};

    // Use POWERSHELL_APP_ID as a reliable fallback — custom app IDs
    // require registration and may fail silently on some Windows versions.
    let _ = Toast::new(Toast::POWERSHELL_APP_ID)
        .title("rusty-sts")
        .text1(&sync_message(imported))
        .duration(Duration::Short)
        .show();
}

#[cfg(not(windows))]
pub fn notify_sync_complete(imported: usize) {
    let _ = notify_rust::Notification::new()
        .summary("rusty-sts")
        .body(&sync_message(imported))
        .timeout(notify_rust::Timeout::Milliseconds(5000))
        .show();
}
