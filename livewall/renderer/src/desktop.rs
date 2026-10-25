//! Attaching windows behind the desktop icons (Progman 0x052C + WorkerW),
//! including the Windows 11 24H2 layout, and monitor enumeration.
use crate::util::{from_wide, wide};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug)]
pub struct Mon {
    pub device: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub primary: bool,
}

pub fn monitors() -> Vec<Mon> {
    let mut v: Vec<Mon> = Vec::new();
    unsafe {
        EnumDisplayMonitors(null_mut(), null(), Some(mon_cb), &mut v as *mut Vec<Mon> as LPARAM);
    }
    v.sort_by(|a, b| a.device.cmp(&b.device));
    v
}

unsafe extern "system" fn mon_cb(hm: HMONITOR, _dc: HDC, _rc: *mut RECT, lp: LPARAM) -> BOOL {
    let v = &mut *(lp as *mut Vec<Mon>);
    let mut mi: MONITORINFOEXW = zeroed();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hm, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) != 0 {
        let r = mi.monitorInfo.rcMonitor;
        v.push(Mon {
            device: from_wide(&mi.szDevice),
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
            primary: mi.monitorInfo.dwFlags & 1 != 0,
        });
    }
    1
}

pub fn virtual_rect(m: &[Mon]) -> (i32, i32, i32, i32) {
    if m.is_empty() {
        return (0, 0, 1920, 1080);
    }
    let x0 = m.iter().map(|a| a.x).min().unwrap();
    let y0 = m.iter().map(|a| a.y).min().unwrap();
    let x1 = m.iter().map(|a| a.x + a.w).max().unwrap();
    let y1 = m.iter().map(|a| a.y + a.h).max().unwrap();
    (x0, y0, x1 - x0, y1 - y0)
}

/// Finds the window that wallpaper windows must be children of.
/// Classic layout (Windows 10, early Windows 11): the WorkerW that follows the
/// WorkerW holding SHELLDLL_DefView. Windows 11 24H2: a WorkerW child of Progman.
pub unsafe fn find_workerw() -> HWND {
    let progman = FindWindowW(wide("Progman").as_ptr(), null());
    if progman.is_null() {
        return null_mut();
    }
    let mut result: usize = 0;
    SendMessageTimeoutW(progman, 0x052C, 0xD, 0x1, 0, 1000, &mut result);

    let mut found: HWND = null_mut();
    EnumWindows(Some(enum_cb), &mut found as *mut HWND as LPARAM);
    if !found.is_null() {
        return found;
    }
    FindWindowExW(progman, null_mut(), wide("WorkerW").as_ptr(), null())
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
    let defview = FindWindowExW(hwnd, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null());
    if !defview.is_null() {
        let ww = FindWindowExW(null_mut(), hwnd, wide("WorkerW").as_ptr(), null());
        if !ww.is_null() {
            *(lp as *mut HWND) = ww;
            return 0;
        }
    }
    1
}

unsafe extern "system" fn plain_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub unsafe fn register_classes() {
    let hinst = GetModuleHandleW(null());
    let surface = wide("LiveWallSurface");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(plain_proc);
    wc.hInstance = hinst;
    wc.lpszClassName = surface.as_ptr();
    wc.hbrBackground = GetStockObject(BLACK_BRUSH) as HBRUSH;
    RegisterClassW(&wc);

    let overlay = wide("LiveWallOverlay");
    let mut wc2: WNDCLASSW = zeroed();
    wc2.lpfnWndProc = Some(plain_proc);
    wc2.hInstance = hinst;
    wc2.lpszClassName = overlay.as_ptr();
    RegisterClassW(&wc2);
}

/// Creates a window and makes it a child of `parent` (the WorkerW).
/// (sx, sy) are screen coordinates. A layered window is click-through.
pub unsafe fn create_child(class: &str, parent: HWND, sx: i32, sy: i32, w: i32, h: i32, layered: bool) -> HWND {
    let cls = wide(class);
    let name = wide("");
    let mut ex = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
    if layered {
        ex |= WS_EX_LAYERED | WS_EX_TRANSPARENT;
    }
    let hwnd = CreateWindowExW(ex, cls.as_ptr(), name.as_ptr(), WS_POPUP, sx, sy, w, h, null_mut(), null_mut(), GetModuleHandleW(null()), null());
    if hwnd.is_null() {
        return hwnd;
    }
    SetParent(hwnd, parent);
    SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN) as isize);
    let mut pt = POINT { x: sx, y: sy };
    ScreenToClient(parent, &mut pt);
    SetWindowPos(hwnd, null_mut(), pt.x, pt.y, w, h, SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    hwnd
}

pub unsafe fn raise(hwnd: HWND) {
    SetWindowPos(hwnd, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}
