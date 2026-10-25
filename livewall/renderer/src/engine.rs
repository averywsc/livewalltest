//! Owns the wallpaper surfaces: one render window and one libmpv player per
//! monitor (or a single one across all monitors).
use crate::desktop::{self, Mon};
use crate::model::{Data, Item, Settings};
use crate::mpv::*;
use crate::pause::Decision;
use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;

pub struct Surface {
    pub device: String,
    pub hwnd: HWND,
    pub mpv: Option<Mpv>,
    pub paused: bool,
    pub span: bool,
    pub audio: bool,
    pub error: String,
}

pub struct Engine {
    pub api: Option<Arc<Api>>,
    pub surfaces: Vec<Surface>,
    pub parent: HWND,
    pub last_error: String,
    wake: unsafe extern "C" fn(*mut c_void),
}

impl Engine {
    pub fn new(api: Option<Arc<Api>>, wake: unsafe extern "C" fn(*mut c_void)) -> Engine {
        Engine { api, surfaces: vec![], parent: std::ptr::null_mut(), last_error: String::new(), wake }
    }

    pub fn destroy_all(&mut self) {
        for mut s in self.surfaces.drain(..) {
            s.mpv.take(); // terminates the player before its window goes away
            unsafe { DestroyWindow(s.hwnd) };
        }
    }

    /// Rebuilds all surfaces from the live scene. Returns false when the desktop
    /// is not ready yet and the caller should retry shortly.
    pub fn rebuild(&mut self, data: &Data) -> bool {
        self.destroy_all();
        self.last_error.clear();
        let wp = &data.live.wallpaper;
        let mons = desktop::monitors();

        struct Job<'a> { device: String, rect: (i32, i32, i32, i32), item: &'a Item, fit: String, span: bool }
        let mut jobs: Vec<Job> = vec![];
        if wp.span {
            if let Some(it) = data.item(&wp.span_item) {
                jobs.push(Job { device: "span".into(), rect: desktop::virtual_rect(&mons), item: it, fit: wp.span_fit.clone(), span: true });
            }
        } else {
            for a in &wp.monitors {
                if let (Some(m), Some(it)) = (mons.iter().find(|m| m.device == a.device), data.item(&a.item)) {
                    jobs.push(Job { device: m.device.clone(), rect: (m.x, m.y, m.w, m.h), item: it, fit: a.fit.clone(), span: false });
                }
            }
        }
        if jobs.is_empty() {
            return true;
        }
        let Some(api) = self.api.clone() else {
            self.last_error = "libmpv-2.dll was not found next to the app, so the wallpaper cannot play.".into();
            return true;
        };
        self.parent = unsafe { desktop::find_workerw() };
        if self.parent.is_null() {
            self.last_error = "The desktop is not ready yet.".into();
            return false;
        }
        let mut audio_taken = false;
        for job in jobs {
            if !Path::new(&job.item.path).exists() {
                self.last_error = format!("File not found: {}", job.item.path);
                continue;
            }
            let (x, y, w, h) = job.rect;
            let hwnd = unsafe { desktop::create_child("LiveWallSurface", self.parent, x, y, w, h, false) };
            if hwnd.is_null() {
                self.last_error = "Could not create the wallpaper window.".into();
                continue;
            }
            let audio = job.item.kind == "video" && !audio_taken;
            match start_player(&api, hwnd, job.item, &job.fit, w, h, &data.settings, audio) {
                Ok(m) => {
                    m.set_wakeup(self.wake);
                    audio_taken |= audio;
                    self.surfaces.push(Surface { device: job.device, hwnd, mpv: Some(m), paused: false, span: job.span, audio, error: String::new() });
                }
                Err(e) => {
                    unsafe { DestroyWindow(hwnd) };
                    self.last_error = e;
                }
            }
        }
        true
    }

    pub fn apply_pause(&mut self, d: &Decision, mons: &[Mon]) {
        for s in &mut self.surfaces {
            let reason = if s.span { d.for_span(mons) } else { d.for_monitor(&s.device) };
            let want = reason.is_some();
            if want != s.paused {
                if let Some(m) = &s.mpv {
                    m.set("pause", if want { "yes" } else { "no" });
                }
                s.paused = want;
            }
        }
    }

    pub fn set_audio(&self, st: &Settings) {
        for s in self.surfaces.iter().filter(|s| s.audio) {
            if let Some(m) = &s.mpv {
                if st.video_muted {
                    m.set("mute", "yes");
                    m.set("aid", "no");
                } else {
                    m.set("aid", "auto");
                    m.set("mute", "no");
                    m.set("volume", &st.video_volume.to_string());
                }
            }
        }
    }

    /// Reads pending player events. Returns true when a player died and the
    /// surfaces should be rebuilt.
    pub fn drain_events(&mut self) -> bool {
        let mut rebuild = false;
        for s in &mut self.surfaces {
            let Some(m) = &s.mpv else { continue };
            loop {
                let ev = m.wait_event(0.0);
                match ev.id {
                    EV_NONE => break,
                    EV_SHUTDOWN => {
                        rebuild = true;
                        break;
                    }
                    EV_END_FILE if ev.end_reason == END_REASON_ERROR => {
                        s.error = "The file could not be played. Its codec may not be supported.".into();
                    }
                    _ => {}
                }
            }
        }
        rebuild
    }

    pub fn errors(&self) -> Vec<String> {
        let mut v = vec![];
        if !self.last_error.is_empty() {
            v.push(self.last_error.clone());
        }
        for s in &self.surfaces {
            if !s.error.is_empty() {
                v.push(s.error.clone());
            }
        }
        v
    }
}

fn start_player(api: &Arc<Api>, hwnd: HWND, item: &Item, fit: &str, mw: i32, mh: i32, st: &Settings, audio: bool) -> Result<Mpv, String> {
    let is_video = item.kind == "video";
    let tile = fit == "tile" && item.width > 0 && item.height > 0;
    let cap_active = is_video && st.fps_cap > 0 && item.fps > st.fps_cap as f64 + 0.5 && !tile;

    let hwdec = if tile { "no" } else if cap_active { "d3d11va-copy" } else { "d3d11va" };
    let mut o: Vec<(&str, String)> = vec![
        ("wid", (hwnd as isize).to_string()),
        ("vo", "gpu".into()),
        ("gpu-api", "d3d11".into()),
        ("gpu-context", "d3d11".into()),
        ("hwdec", hwdec.into()),
        ("config", "no".into()),
        ("load-scripts", "no".into()),
        ("ytdl", "no".into()),
        ("terminal", "no".into()),
        ("osc", "no".into()),
        ("osd-level", "0".into()),
        ("input-default-bindings", "no".into()),
        ("input-vo-keyboard", "no".into()),
        ("input-cursor", "no".into()),
        ("cursor-autohide", "no".into()),
        ("border", "no".into()),
        ("background-color", "#000000".into()),
        ("force-window", "yes".into()),
        ("scale", "bilinear".into()),
        ("cscale", "bilinear".into()),
        ("dscale", "bilinear".into()),
        ("correct-downscaling", "no".into()),
        ("sigmoid-upscaling", "no".into()),
        ("deband", "no".into()),
        ("dither-depth", "no".into()),
        ("interpolation", "no".into()),
        ("video-sync", "audio".into()),
        ("cache", "no".into()),
        ("demuxer-max-bytes", "48MiB".into()),
        ("demuxer-max-back-bytes", "16MiB".into()),
        ("vd-lavc-threads", "2".into()),
        ("image-display-duration", "inf".into()),
    ];
    if item.kind != "image" {
        o.push(("loop-file", "inf".into()));
    } else {
        o.push(("keep-open", "yes".into()));
    }
    if is_video && audio && !st.video_muted {
        o.push(("mute", "no".into()));
        o.push(("volume", st.video_volume.to_string()));
    } else {
        o.push(("mute", "yes".into()));
        o.push(("aid", "no".into()));
    }
    if cap_active {
        o.push(("vf", format!("fps={}", st.fps_cap)));
    }
    match fit {
        "fit" => o.push(("panscan", "0".into())),
        "stretch" => o.push(("keepaspect", "no".into())),
        "center" => {
            o.push(("video-unscaled", "downscale-big".into()));
            o.push(("panscan", "0".into()));
        }
        "tile" if tile => {
            o.push(("video-unscaled", "yes".into()));
            o.push(("panscan", "0".into()));
            o.push(("aid", "no".into()));
            o.push(("lavfi-complex", tile_graph(item.width, item.height, mw, mh)));
        }
        _ => o.push(("panscan", "1.0".into())),
    }

    let m = Mpv::new(api, &o)?;
    m.command(&["loadfile", &item.path, "replace"])?;
    Ok(m)
}

/// Builds an ffmpeg filter graph that repeats the picture until it covers the monitor.
/// Tiling runs on the CPU, so it is meant for images and short GIFs.
fn tile_graph(iw: u32, ih: u32, mw: i32, mh: i32) -> String {
    let (mw, mh) = (mw.max(1) as u32, mh.max(1) as u32);
    let (iw, ih) = (iw.max(1), ih.max(1));
    let mut f = 1u32;
    loop {
        let cols = (mw + iw * f - 1) / (iw * f);
        let rows = (mh + ih * f - 1) / (ih * f);
        if cols * rows <= 256 || f >= 64 {
            break;
        }
        f += 1;
    }
    let (cw, ch) = (iw * f, ih * f);
    let cols = (mw + cw - 1) / cw;
    let rows = (mh + ch - 1) / ch;
    let n = cols * rows;
    let scale = if f > 1 { format!("scale={cw}:{ch}:flags=neighbor,") } else { String::new() };

    if n == 1 {
        return format!("[vid1]{scale}crop=w='min(iw,{mw})':h='min(ih,{mh})':x=0:y=0[vo]");
    }
    let mut g = format!("[vid1]{scale}split={n}");
    for i in 0..n {
        g += &format!("[t{i}]");
    }
    g.push(';');
    for r in 0..rows {
        if cols == 1 {
            g += &format!("[t{r}]null[r{r}];");
        } else {
            for c in 0..cols {
                g += &format!("[t{}]", r * cols + c);
            }
            g += &format!("hstack=inputs={cols}[r{r}];");
        }
    }
    if rows == 1 {
        g += &format!("[r0]crop={mw}:{mh}:0:0[vo]");
    } else {
        for r in 0..rows {
            g += &format!("[r{r}]");
        }
        g += &format!("vstack=inputs={rows},crop={mw}:{mh}:0:0[vo]");
    }
    g
}
