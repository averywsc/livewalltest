#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine as _;
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};

const PIPE: &str = r"\\.\pipe\livewall-ctl";

struct ChildArgs(Vec<String>);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Sends one JSON request to the renderer and returns its JSON reply.
/// Returns Err("offline") when the renderer is not running.
#[tauri::command]
fn rpc(request: String) -> Result<String, String> {
    for _ in 0..40 {
        match std::fs::OpenOptions::new().read(true).write(true).open(PIPE) {
            Ok(mut f) => {
                f.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
                f.write_all(b"\n").map_err(|e| e.to_string())?;
                let mut line = String::new();
                BufReader::new(&f).read_line(&mut line).map_err(|e| e.to_string())?;
                return Ok(line);
            }
            Err(e) => match e.raw_os_error() {
                Some(2) => return Err("offline".into()),
                Some(231) => std::thread::sleep(Duration::from_millis(25)),
                _ => return Err(e.to_string()),
            },
        }
    }
    Err("The renderer is busy. Try again.".into())
}

#[tauri::command]
fn start_renderer(args: tauri::State<ChildArgs>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let renderer = exe.parent().ok_or("no folder")?.join("livewall.exe");
    std::process::Command::new(renderer).args(&args.0).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Returns a thumbnail as a data URL. Only PNG and JPEG files are read.
#[tauri::command]
fn read_data_url(path: String) -> Result<String, String> {
    let lower = path.to_lowercase();
    let mime = if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        return Err("unsupported".into());
    };
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn main() {
    // One settings window at a time: bring the existing one forward.
    unsafe {
        let _m = CreateMutexW(std::ptr::null(), 0, wide("LiveWallSettingsWindow").as_ptr());
        if GetLastError() == 183 {
            let h = FindWindowW(std::ptr::null(), wide("Live Wallpaper").as_ptr());
            if !h.is_null() {
                ShowWindow(h, SW_RESTORE);
                SetForegroundWindow(h);
            }
            return;
        }
        std::mem::forget(_m);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ChildArgs(args))
        .invoke_handler(tauri::generate_handler![rpc, start_renderer, read_data_url])
        .run(tauri::generate_context!())
        .expect("the settings window could not start");
}
