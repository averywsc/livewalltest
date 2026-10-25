use crate::util::wide;
use std::mem::{size_of, zeroed};
use std::ptr::null;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const ID_OPEN: u32 = 1;
pub const ID_PAUSE: u32 = 2;
pub const ID_MUTE: u32 = 3;
pub const ID_MUSIC_TOGGLE: u32 = 10;
pub const ID_MUSIC_NEXT: u32 = 11;
pub const ID_MUSIC_PREV: u32 = 12;
pub const ID_SCENE_BASE: u32 = 100;
pub const ID_QUIT: u32 = 999;

pub struct MenuModel {
    pub paused_by_user: bool,
    pub muted: bool,
    pub music_playing: bool,
    pub scenes: Vec<(String, String)>, // id, name
    pub current_scene: String,
}

unsafe fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut n: NOTIFYICONDATAW = zeroed();
    n.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    n.hWnd = hwnd;
    n.uID = 1;
    n
}

pub unsafe fn add(hwnd: HWND, callback_msg: u32) {
    let mut n = nid(hwnd);
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = callback_msg;
    let size = GetSystemMetrics(SM_CXSMICON);
    let mut icon = LoadImageW(GetModuleHandleW(null()), 1usize as *const u16, IMAGE_ICON, size, size, 0);
    if icon.is_null() {
        icon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as _;
    }
    n.hIcon = icon as _;
    let tip = wide("Live Wallpaper");
    for (i, c) in tip.iter().take(127).enumerate() {
        n.szTip[i] = *c;
    }
    Shell_NotifyIconW(NIM_DELETE, &n); // clears a stale icon after an Explorer restart
    Shell_NotifyIconW(NIM_ADD, &n);
}

pub unsafe fn remove(hwnd: HWND) {
    let n = nid(hwnd);
    Shell_NotifyIconW(NIM_DELETE, &n);
}

unsafe fn item(menu: HMENU, id: u32, text: &str, checked: bool) {
    let flags = MF_STRING | if checked { MF_CHECKED } else { 0 };
    AppendMenuW(menu, flags, id as usize, wide(text).as_ptr());
}

/// Shows the tray menu and returns the chosen command id (0 when dismissed).
pub unsafe fn show_menu(hwnd: HWND, m: &MenuModel) -> u32 {
    let menu = CreatePopupMenu();
    item(menu, ID_OPEN, "Open settings", false);
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    item(menu, ID_PAUSE, "Pause wallpaper", m.paused_by_user);
    item(menu, ID_MUTE, "Mute video sound", m.muted);

    let music = CreatePopupMenu();
    item(music, ID_MUSIC_TOGGLE, if m.music_playing { "Pause" } else { "Play" }, false);
    item(music, ID_MUSIC_NEXT, "Next track", false);
    item(music, ID_MUSIC_PREV, "Previous track", false);
    AppendMenuW(menu, MF_POPUP, music as usize, wide("Music").as_ptr());

    let scenes = CreatePopupMenu();
    if m.scenes.is_empty() {
        AppendMenuW(scenes, MF_STRING | MF_GRAYED, 0, wide("No saved scenes").as_ptr());
    }
    for (i, (id, name)) in m.scenes.iter().enumerate() {
        item(scenes, ID_SCENE_BASE + i as u32, name, *id == m.current_scene);
    }
    AppendMenuW(menu, MF_POPUP, scenes as usize, wide("Scenes").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    item(menu, ID_QUIT, "Quit", false);

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    cmd as u32
}
