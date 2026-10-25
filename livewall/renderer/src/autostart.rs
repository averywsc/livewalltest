use crate::util::wide;
use std::ptr::null_mut;
use windows_sys::Win32::System::Registry::*;

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const NAME: &str = "LiveWallpaper";

/// Adds or removes the per-user Run entry. `command` is the full command line to store.
pub fn set(enabled: bool, command: &str) {
    unsafe {
        let mut key: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
            return;
        }
        let name = wide(NAME);
        if enabled {
            let v = wide(command);
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32);
        } else {
            RegDeleteValueW(key, name.as_ptr());
        }
        RegCloseKey(key);
    }
}
