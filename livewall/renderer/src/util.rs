use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

pub fn exe_dir() -> PathBuf {
    exe_path().parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

pub fn ext(p: &str) -> String {
    Path::new(p).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn stem(p: &str) -> String {
    Path::new(p).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Returns "video", "gif" or "image" for supported wallpaper/overlay files.
pub fn classify(p: &str) -> Option<&'static str> {
    match ext(p).as_str() {
        "mp4" | "mkv" | "webm" | "mov" | "avi" => Some("video"),
        "gif" => Some("gif"),
        "png" | "jpg" | "jpeg" | "bmp" | "webp" => Some("image"),
        _ => None,
    }
}

pub fn is_audio(p: &str) -> bool {
    matches!(ext(p).as_str(), "mp3" | "flac" | "wav" | "ogg" | "m4a")
}

pub fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    format!("{:x}{:x}", t, N.fetch_add(1, Ordering::Relaxed))
}
