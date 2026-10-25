//! Thin dynamic binding to libmpv (loaded at run time from libmpv-2.dll).
use crate::util::exe_dir;
use libloading::Library;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::null_mut;
use std::sync::Arc;

pub type Handle = *mut c_void;

pub const EV_NONE: i32 = 0;
pub const EV_SHUTDOWN: i32 = 1;
pub const EV_END_FILE: i32 = 7;
pub const EV_FILE_LOADED: i32 = 8;
pub const EV_VIDEO_RECONFIG: i32 = 17;
pub const EV_PLAYBACK_RESTART: i32 = 21;
pub const END_REASON_ERROR: i32 = 4;

#[repr(C)]
struct RawEvent {
    id: c_int,
    error: c_int,
    reply: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEnd {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct RenderParam {
    typ: c_int,
    data: *mut c_void,
}

pub struct Ev {
    pub id: i32,
    pub end_reason: i32,
}

type WakeCb = unsafe extern "C" fn(*mut c_void);

pub struct Api {
    _lib: Library,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(Handle),
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char,
    free: unsafe extern "C" fn(*mut c_void),
    command: unsafe extern "C" fn(Handle, *mut *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, f64) -> *mut RawEvent,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    set_wakeup_callback: unsafe extern "C" fn(Handle, Option<WakeCb>, *mut c_void),
    render_create: unsafe extern "C" fn(*mut *mut c_void, Handle, *mut RenderParam) -> c_int,
    render_update: unsafe extern "C" fn(*mut c_void) -> u64,
    render_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(*mut c_void),
}

impl Api {
    pub fn load() -> Result<Arc<Api>, String> {
        let candidates = [exe_dir().join("libmpv-2.dll"), "libmpv-2.dll".into()];
        let mut last = String::new();
        for c in candidates.iter() {
            match unsafe { Library::new(c) } {
                Ok(lib) => return unsafe { Self::bind(lib) }.map(Arc::new),
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!("libmpv-2.dll could not be loaded: {last}"))
    }

    unsafe fn bind(lib: Library) -> Result<Api, String> {
        let e = |x: libloading::Error| x.to_string();
        Ok(Api {
            create: *lib.get(b"mpv_create\0").map_err(e)?,
            initialize: *lib.get(b"mpv_initialize\0").map_err(e)?,
            terminate_destroy: *lib.get(b"mpv_terminate_destroy\0").map_err(e)?,
            set_option_string: *lib.get(b"mpv_set_option_string\0").map_err(e)?,
            set_property_string: *lib.get(b"mpv_set_property_string\0").map_err(e)?,
            get_property_string: *lib.get(b"mpv_get_property_string\0").map_err(e)?,
            free: *lib.get(b"mpv_free\0").map_err(e)?,
            command: *lib.get(b"mpv_command\0").map_err(e)?,
            wait_event: *lib.get(b"mpv_wait_event\0").map_err(e)?,
            error_string: *lib.get(b"mpv_error_string\0").map_err(e)?,
            set_wakeup_callback: *lib.get(b"mpv_set_wakeup_callback\0").map_err(e)?,
            render_create: *lib.get(b"mpv_render_context_create\0").map_err(e)?,
            render_update: *lib.get(b"mpv_render_context_update\0").map_err(e)?,
            render_render: *lib.get(b"mpv_render_context_render\0").map_err(e)?,
            render_free: *lib.get(b"mpv_render_context_free\0").map_err(e)?,
            _lib: lib,
        })
    }
}

pub struct Mpv {
    api: Arc<Api>,
    h: Handle,
    render: *mut c_void,
}

unsafe impl Send for Mpv {}

impl Mpv {
    /// Creates and initialises a player. Unknown options are ignored so that
    /// older or smaller libmpv builds still start.
    pub fn new(api: &Arc<Api>, opts: &[(&str, String)]) -> Result<Mpv, String> {
        unsafe {
            let h = (api.create)();
            if h.is_null() {
                return Err("mpv_create failed".into());
            }
            for (k, v) in opts {
                let (Ok(ck), Ok(cv)) = (CString::new(*k), CString::new(v.as_str())) else { continue };
                (api.set_option_string)(h, ck.as_ptr(), cv.as_ptr());
            }
            let r = (api.initialize)(h);
            if r < 0 {
                let msg = CStr::from_ptr((api.error_string)(r)).to_string_lossy().to_string();
                (api.terminate_destroy)(h);
                return Err(format!("mpv_initialize failed: {msg}"));
            }
            Ok(Mpv { api: api.clone(), h, render: null_mut() })
        }
    }

    /// Player that renders into memory (software render API); used for video overlays.
    pub fn new_sw(api: &Arc<Api>, opts: &[(&str, String)]) -> Result<Mpv, String> {
        let mut m = Self::new(api, opts)?;
        unsafe {
            let kind = CString::new("sw").unwrap();
            let mut params = [
                RenderParam { typ: 1, data: kind.as_ptr() as *mut c_void },
                RenderParam { typ: 0, data: null_mut() },
            ];
            let mut ctx: *mut c_void = null_mut();
            let r = (api.render_create)(&mut ctx, m.h, params.as_mut_ptr());
            if r < 0 || ctx.is_null() {
                return Err("could not create the mpv software renderer".into());
            }
            m.render = ctx;
        }
        Ok(m)
    }

    pub fn set(&self, k: &str, v: &str) {
        if let (Ok(ck), Ok(cv)) = (CString::new(k), CString::new(v)) {
            unsafe { (self.api.set_property_string)(self.h, ck.as_ptr(), cv.as_ptr()) };
        }
    }

    pub fn get(&self, k: &str) -> Option<String> {
        let ck = CString::new(k).ok()?;
        unsafe {
            let p = (self.api.get_property_string)(self.h, ck.as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().to_string();
            (self.api.free)(p as *mut c_void);
            Some(s)
        }
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().filter_map(|a| CString::new(*a).ok()).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let r = unsafe { (self.api.command)(self.h, ptrs.as_mut_ptr()) };
        if r < 0 {
            let msg = unsafe { CStr::from_ptr((self.api.error_string)(r)).to_string_lossy().to_string() };
            Err(msg)
        } else {
            Ok(())
        }
    }

    pub fn wait_event(&self, timeout: f64) -> Ev {
        unsafe {
            let ev = (self.api.wait_event)(self.h, timeout);
            if ev.is_null() {
                return Ev { id: EV_NONE, end_reason: 0 };
            }
            let id = (*ev).id;
            let mut end_reason = 0;
            if id == EV_END_FILE && !(*ev).data.is_null() {
                end_reason = (*((*ev).data as *const RawEnd)).reason;
            }
            Ev { id, end_reason }
        }
    }

    pub fn set_wakeup(&self, cb: WakeCb) {
        unsafe { (self.api.set_wakeup_callback)(self.h, Some(cb), null_mut()) };
    }

    /// Returns true when mpv has a new frame ready to be rendered.
    pub fn render_has_frame(&self) -> bool {
        if self.render.is_null() {
            return false;
        }
        unsafe { (self.api.render_update)(self.render) & 1 != 0 }
    }

    /// Renders the current frame as BGR0 into `buf` (w*h*4 bytes).
    pub fn render_sw(&self, w: i32, h: i32, buf: &mut [u8]) -> bool {
        if self.render.is_null() || buf.len() < (w * h * 4) as usize {
            return false;
        }
        let mut size = [w as c_int, h as c_int];
        let fmt = CString::new("bgr0").unwrap();
        let mut stride: usize = (w * 4) as usize;
        let mut params = [
            RenderParam { typ: 17, data: size.as_mut_ptr() as *mut c_void },
            RenderParam { typ: 18, data: fmt.as_ptr() as *mut c_void },
            RenderParam { typ: 19, data: &mut stride as *mut usize as *mut c_void },
            RenderParam { typ: 20, data: buf.as_mut_ptr() as *mut c_void },
            RenderParam { typ: 0, data: null_mut() },
        ];
        unsafe { (self.api.render_render)(self.render, params.as_mut_ptr()) >= 0 }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            if !self.render.is_null() {
                (self.api.render_free)(self.render);
            }
            (self.api.terminate_destroy)(self.h);
        }
    }
}
