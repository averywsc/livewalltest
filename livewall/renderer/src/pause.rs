//! Decides when rendering must stop. No polling loops: the caller re-evaluates
//! on window events, power events and a slow safety timer.
use crate::desktop::Mon;
use crate::model::Settings;
use crate::util::from_wide;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Default, Clone)]
pub struct Flags {
    pub locked: bool,
    pub display_off: bool,
    pub asleep: bool,
    pub manual: bool,
}

#[derive(Default)]
pub struct Decision {
    pub global: Option<&'static str>,
    pub fullscreen: Vec<String>,
    pub maximized: Vec<String>,
}

impl Decision {
    pub fn for_monitor(&self, dev: &str) -> Option<&'static str> {
        if let Some(g) = self.global {
            return Some(g);
        }
        if self.fullscreen.iter().any(|d| d == dev) {
            return Some("a game or fullscreen app is open");
        }
        if self.maximized.iter().any(|d| d == dev) {
            return Some("a window is maximized");
        }
        None
    }

    /// A wallpaper stretched over all monitors pauses on any fullscreen app,
    /// and when every monitor has a maximized window.
    pub fn for_span(&self, mons: &[Mon]) -> Option<&'static str> {
        if let Some(g) = self.global {
            return Some(g);
        }
        if !self.fullscreen.is_empty() {
            return Some("a game or fullscreen app is open");
        }
        if !mons.is_empty() && mons.iter().all(|m| self.maximized.contains(&m.device)) {
            return Some("a window is maximized");
        }
        None
    }

    pub fn for_point(&self, mons: &[Mon], x: f64, y: f64) -> Option<&'static str> {
        if let Some(g) = self.global {
            return Some(g);
        }
        let m = mons.iter().find(|m| x >= m.x as f64 && x < (m.x + m.w) as f64 && y >= m.y as f64 && y < (m.y + m.h) as f64)?;
        self.for_monitor(&m.device)
    }
}

pub fn evaluate(s: &Settings, f: &Flags) -> Decision {
    let mut d = Decision::default();
    unsafe {
        d.global = if f.manual {
            Some("paused by you")
        } else if f.locked {
            Some("the screen is locked")
        } else if f.display_off {
            Some("the screen is off")
        } else if f.asleep {
            Some("the computer is asleep")
        } else if s.pause_battery && on_battery() {
            Some("the computer is running on battery")
        } else if s.pause_fullscreen && exclusive_fullscreen() {
            Some("a game or fullscreen app is open")
        } else {
            None
        };
        if d.global.is_none() {
            if s.pause_fullscreen {
                if let Some(dev) = fullscreen_monitor() {
                    d.fullscreen.push(dev);
                }
            }
            if s.pause_maximized {
                d.maximized = maximized_monitors();
            }
        }
    }
    d
}

unsafe fn on_battery() -> bool {
    let mut s: SYSTEM_POWER_STATUS = zeroed();
    GetSystemPowerStatus(&mut s) != 0 && s.ACLineStatus == 0
}

/// Direct3D exclusive fullscreen or presentation mode.
unsafe fn exclusive_fullscreen() -> bool {
    let mut st: i32 = 0;
    if SHQueryUserNotificationState(&mut st) != 0 {
        return false;
    }
    st == 3 || st == 4
}

unsafe fn class_of(h: HWND) -> String {
    let mut b = [0u16; 96];
    let n = GetClassNameW(h, b.as_mut_ptr(), 96);
    from_wide(&b[..n.max(0) as usize])
}

fn is_shell_class(c: &str) -> bool {
    matches!(
        c,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "XamlExplorerHostIslandWindow"
            | "Windows.UI.Core.CoreWindow" | "TopLevelWindowForOverflowXamlIsland" | "NotifyIconOverflowWindow"
    )
}

unsafe fn cloaked(h: HWND) -> bool {
    let mut v: u32 = 0;
    DwmGetWindowAttribute(h, 14, &mut v as *mut u32 as *mut c_void, 4) == 0 && v != 0
}

unsafe fn monitor_of(h: HWND) -> Option<(String, RECT)> {
    let hm = MonitorFromWindow(h, 2);
    if hm.is_null() {
        return None;
    }
    let mut mi: MONITORINFOEXW = zeroed();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hm, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) == 0 {
        return None;
    }
    Some((from_wide(&mi.szDevice), mi.monitorInfo.rcMonitor))
}

unsafe fn fullscreen_monitor() -> Option<String> {
    let fg = GetForegroundWindow();
    if fg.is_null() || IsWindowVisible(fg) == 0 || IsIconic(fg) != 0 || cloaked(fg) {
        return None;
    }
    if is_shell_class(&class_of(fg)) {
        return None;
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(fg, &mut r) == 0 {
        return None;
    }
    let (dev, m) = monitor_of(fg)?;
    if r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom {
        Some(dev)
    } else {
        None
    }
}

unsafe fn maximized_monitors() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    EnumWindows(Some(enum_max), &mut out as *mut Vec<String> as LPARAM);
    out
}

unsafe extern "system" fn enum_max(h: HWND, lp: LPARAM) -> BOOL {
    if IsWindowVisible(h) == 0 || IsIconic(h) != 0 || IsZoomed(h) == 0 || cloaked(h) {
        return 1;
    }
    if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 || is_shell_class(&class_of(h)) {
        return 1;
    }
    if let Some((dev, _)) = monitor_of(h) {
        let out = &mut *(lp as *mut Vec<String>);
        if !out.contains(&dev) {
            out.push(dev);
        }
    }
    1
}
