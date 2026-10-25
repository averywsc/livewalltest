//! Control channel between the settings window and the renderer: one JSON
//! request per line over a local named pipe (no network involved).
use crate::model::*;
use crate::mpv::Api;
use crate::music::{MCmd, MusicStatus};
use crate::{desktop, store, thumb, util};
use serde_json::{json, Value};
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const PIPE_NAME: &str = r"\\.\pipe\livewall-ctl";

pub enum Cmd {
    Wallpaper,
    Overlays,
    Settings,
    Scene,
    PauseToggle,
    OpenSettings,
    Quit,
}

pub struct Ctx {
    pub data: Arc<Mutex<Data>>,
    pub rt: Arc<Mutex<Runtime>>,
    pub tx: Sender<Cmd>,
    pub api: Option<Arc<Api>>,
    pub music: Sender<MCmd>,
    pub music_st: Arc<Mutex<MusicStatus>>,
}

impl Ctx {
    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        let h = crate::CTRL.load(Ordering::Relaxed);
        if h != 0 {
            unsafe { PostMessageW(h as HWND, crate::WM_APP_CMD, 0, 0) };
        }
    }
}

pub fn spawn(ctx: Ctx) {
    std::thread::spawn(move || serve(ctx));
}

fn serve(ctx: Ctx) {
    let name = util::wide(PIPE_NAME);
    // 3 = duplex, 8 = reject remote clients, one instance only.
    let pipe = unsafe { CreateNamedPipeW(name.as_ptr(), 3, 8, 1, 1 << 20, 1 << 16, 0, null()) };
    if pipe == INVALID_HANDLE_VALUE {
        return;
    }
    loop {
        let connected = unsafe { ConnectNamedPipe(pipe, null_mut()) != 0 || GetLastError() == 535 };
        if connected {
            if let Some(line) = read_line(pipe) {
                let resp = match serde_json::from_str::<Value>(&line) {
                    Ok(req) => handle(&ctx, &req),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                };
                let mut s = resp.to_string();
                s.push('\n');
                write_all(pipe, s.as_bytes());
            }
            unsafe { FlushFileBuffers(pipe) };
        }
        unsafe { DisconnectNamedPipe(pipe) };
    }
}

fn read_line(pipe: HANDLE) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut n: u32 = 0;
        let ok = unsafe { ReadFile(pipe, chunk.as_mut_ptr() as _, chunk.len() as u32, &mut n, null_mut()) };
        if ok == 0 || n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        if buf.contains(&b'\n') || buf.len() > (4 << 20) {
            break;
        }
    }
    let end = buf.iter().position(|&b| b == b'\n').unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

fn write_all(pipe: HANDLE, mut data: &[u8]) {
    while !data.is_empty() {
        let mut n: u32 = 0;
        let ok = unsafe { WriteFile(pipe, data.as_ptr() as _, data.len() as u32, &mut n, null_mut()) };
        if ok == 0 || n == 0 {
            return;
        }
        data = &data[n as usize..];
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn err(msg: &str) -> Value {
    json!({"ok": false, "error": msg})
}

fn ok() -> Value {
    json!({"ok": true})
}

fn handle(ctx: &Ctx, req: &Value) -> Value {
    let cmd = req["cmd"].as_str().unwrap_or("");
    match cmd {
        "get_state" => {
            let d = ctx.data.lock().unwrap();
            let rt = ctx.rt.lock().unwrap();
            let ms = ctx.music_st.lock().unwrap();
            json!({
                "ok": true,
                "library": d.library,
                "settings": d.settings,
                "live": d.live,
                "scenes": d.scenes.iter().map(|s| json!({"id": s.id, "name": s.name})).collect::<Vec<_>>(),
                "current_scene": d.current_scene,
                "runtime": *rt,
                "music": *ms,
            })
        }

        "add_library" => {
            let mut added = vec![];
            for p in strings(&req["paths"]) {
                let Some(kind) = util::classify(&p) else { continue };
                if ctx.data.lock().unwrap().library.iter().any(|i| i.path.eq_ignore_ascii_case(&p)) {
                    continue;
                }
                let id = util::new_id();
                let pr = thumb::make(ctx.api.as_ref(), &p, kind, &id);
                let item = Item { id, path: p.clone(), name: util::stem(&p), kind: kind.into(), thumb: pr.thumb, fps: pr.fps, duration: pr.duration, width: pr.w, height: pr.h };
                let mut d = ctx.data.lock().unwrap();
                d.library.push(item.clone());
                store::save(&d);
                added.push(item);
            }
            json!({"ok": true, "added": added})
        }

        "rename_item" => {
            let (id, name) = (req["id"].as_str().unwrap_or(""), req["name"].as_str().unwrap_or("").trim());
            if name.is_empty() {
                return err("The name cannot be empty.");
            }
            let mut d = ctx.data.lock().unwrap();
            if let Some(i) = d.library.iter_mut().find(|i| i.id == id) {
                i.name = name.to_string();
            }
            store::save(&d);
            ok()
        }

        "remove_item" => {
            let id = req["id"].as_str().unwrap_or("").to_string();
            let mut d = ctx.data.lock().unwrap();
            let in_use = d.live.wallpaper.span_item == id || d.live.wallpaper.monitors.iter().any(|m| m.item == id);
            if let Some(pos) = d.library.iter().position(|i| i.id == id) {
                let it = d.library.remove(pos);
                if !it.thumb.is_empty() {
                    let _ = std::fs::remove_file(&it.thumb);
                }
            }
            if d.live.wallpaper.span_item == id {
                d.live.wallpaper.span_item.clear();
            }
            d.live.wallpaper.monitors.retain(|m| m.item != id);
            store::save(&d);
            drop(d);
            if in_use {
                ctx.send(Cmd::Wallpaper);
            }
            ok()
        }

        "set_wallpaper" => match serde_json::from_value::<Wallpaper>(req["wallpaper"].clone()) {
            Ok(w) => {
                let mut d = ctx.data.lock().unwrap();
                d.live.wallpaper = w;
                store::save(&d);
                drop(d);
                ctx.send(Cmd::Wallpaper);
                ok()
            }
            Err(e) => err(&e.to_string()),
        },

        "add_overlay" => {
            let mut added = vec![];
            for p in strings(&req["paths"]) {
                let Some(kind) = util::classify(&p) else { continue };
                let id = util::new_id();
                let pr = thumb::make(ctx.api.as_ref(), &p, kind, &id);
                let mons = desktop::monitors();
                let pm = mons.iter().find(|m| m.primary).or(mons.first());
                let (cx, cy, mh) = pm
                    .map(|m| (m.x as f64 + m.w as f64 / 2.0, m.y as f64 + m.h as f64 / 2.0, m.h as f64))
                    .unwrap_or((960.0, 540.0, 1080.0));
                let (nw, nh) = if pr.w > 0 && pr.h > 0 { (pr.w as f64, pr.h as f64) } else { (480.0, 270.0) };
                let scale = (mh * 0.3 / nh).min(1.0);
                let o = Overlay {
                    id, path: p.clone(), name: util::stem(&p), kind: kind.into(), thumb: pr.thumb,
                    cx, cy, w: (nw * scale).max(32.0), h: (nh * scale).max(32.0), rotation: 0.0, opacity: 1.0, nw: pr.w, nh: pr.h,
                };
                let mut d = ctx.data.lock().unwrap();
                d.live.overlays.push(o.clone());
                store::save(&d);
                added.push(o);
            }
            ctx.send(Cmd::Overlays);
            json!({"ok": true, "added": added})
        }

        "set_overlay" => {
            let id = req["id"].as_str().unwrap_or("");
            let mut d = ctx.data.lock().unwrap();
            if let Some(o) = d.live.overlays.iter_mut().find(|o| o.id == id) {
                if let Some(v) = req["cx"].as_f64() { o.cx = v; }
                if let Some(v) = req["cy"].as_f64() { o.cy = v; }
                if let Some(v) = req["w"].as_f64() { o.w = v.max(8.0); }
                if let Some(v) = req["h"].as_f64() { o.h = v.max(8.0); }
                if let Some(v) = req["rotation"].as_f64() { o.rotation = v; }
                if let Some(v) = req["opacity"].as_f64() { o.opacity = v.clamp(0.0, 1.0); }
            }
            if req["commit"].as_bool().unwrap_or(false) {
                store::save(&d);
            }
            drop(d);
            ctx.send(Cmd::Overlays);
            ok()
        }

        "remove_overlay" => {
            let id = req["id"].as_str().unwrap_or("").to_string();
            let mut d = ctx.data.lock().unwrap();
            if let Some(pos) = d.live.overlays.iter().position(|o| o.id == id) {
                let o = d.live.overlays.remove(pos);
                let used = d.scenes.iter().any(|s| s.overlays.iter().any(|x| x.thumb == o.thumb));
                if !o.thumb.is_empty() && !used {
                    let _ = std::fs::remove_file(&o.thumb);
                }
            }
            store::save(&d);
            drop(d);
            ctx.send(Cmd::Overlays);
            ok()
        }

        "music_add" => {
            let mut d = ctx.data.lock().unwrap();
            for p in strings(&req["paths"]) {
                if util::is_audio(&p) && !d.live.music.playlist.iter().any(|t| t.path.eq_ignore_ascii_case(&p)) {
                    d.live.music.playlist.push(Track { name: util::stem(&p), path: p });
                }
            }
            store::save(&d);
            let paths: Vec<String> = d.live.music.playlist.iter().map(|t| t.path.clone()).collect();
            let _ = ctx.music.send(MCmd::SetList(paths));
            ok()
        }

        "music_remove" => {
            let i = req["index"].as_u64().unwrap_or(u64::MAX) as usize;
            let mut d = ctx.data.lock().unwrap();
            if i < d.live.music.playlist.len() {
                d.live.music.playlist.remove(i);
            }
            store::save(&d);
            let paths: Vec<String> = d.live.music.playlist.iter().map(|t| t.path.clone()).collect();
            let _ = ctx.music.send(MCmd::SetList(paths));
            ok()
        }

        "music_ctl" => {
            let action = req["action"].as_str().unwrap_or("");
            let mut d = ctx.data.lock().unwrap();
            match action {
                "play" => { let _ = ctx.music.send(MCmd::Play); d.live.music.autoplay = true; }
                "pause" => { let _ = ctx.music.send(MCmd::Pause); d.live.music.autoplay = false; }
                "toggle" => {
                    let _ = ctx.music.send(MCmd::Toggle);
                    let st = ctx.music_st.lock().unwrap();
                    d.live.music.autoplay = !(st.playing && !st.paused);
                }
                "next" => { let _ = ctx.music.send(MCmd::Next); d.live.music.autoplay = true; }
                "prev" => { let _ = ctx.music.send(MCmd::Prev); d.live.music.autoplay = true; }
                "select" => {
                    let i = req["index"].as_u64().unwrap_or(0) as usize;
                    let _ = ctx.music.send(MCmd::Select(i));
                    d.live.music.current = i;
                    d.live.music.autoplay = true;
                }
                _ => return err("Unknown music action."),
            }
            store::save(&d);
            ok()
        }

        "music_set" => {
            let mut d = ctx.data.lock().unwrap();
            if let Some(v) = req["volume"].as_u64() {
                d.live.music.volume = v.min(100) as u32;
                let _ = ctx.music.send(MCmd::Volume(d.live.music.volume));
            }
            if let Some(b) = req["shuffle"].as_bool() {
                d.live.music.shuffle = b;
                let _ = ctx.music.send(MCmd::Shuffle(b));
            }
            if let Some(m) = req["loop_mode"].as_str() {
                if matches!(m, "off" | "all" | "one") {
                    d.live.music.loop_mode = m.to_string();
                    let _ = ctx.music.send(MCmd::LoopMode(m.to_string()));
                }
            }
            if req["commit"].as_bool().unwrap_or(true) {
                store::save(&d);
            }
            ok()
        }

        "set_settings" => match serde_json::from_value::<Settings>(req["settings"].clone()) {
            Ok(s) => {
                let mut d = ctx.data.lock().unwrap();
                d.settings = s;
                store::save(&d);
                drop(d);
                ctx.send(Cmd::Settings);
                ok()
            }
            Err(e) => err(&e.to_string()),
        },

        "scene_save" => {
            let name = req["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return err("Enter a name for the scene.");
            }
            let st = ctx.music_st.lock().unwrap().clone();
            let mut d = ctx.data.lock().unwrap();
            let mut s = d.live.clone();
            s.id = util::new_id();
            s.name = name;
            s.music.autoplay = st.playing && !st.paused;
            s.music.current = st.index;
            d.current_scene = s.id.clone();
            d.scenes.push(s);
            store::save(&d);
            ok()
        }

        "scene_load" => {
            let id = req["id"].as_str().unwrap_or("");
            let mut d = ctx.data.lock().unwrap();
            if !d.load_scene(id) {
                return err("That scene no longer exists.");
            }
            store::save(&d);
            drop(d);
            ctx.send(Cmd::Scene);
            ok()
        }

        "scene_rename" => {
            let (id, name) = (req["id"].as_str().unwrap_or(""), req["name"].as_str().unwrap_or("").trim());
            if name.is_empty() {
                return err("The name cannot be empty.");
            }
            let mut d = ctx.data.lock().unwrap();
            if let Some(s) = d.scenes.iter_mut().find(|s| s.id == id) {
                s.name = name.to_string();
            }
            store::save(&d);
            ok()
        }

        "scene_delete" => {
            let id = req["id"].as_str().unwrap_or("");
            let mut d = ctx.data.lock().unwrap();
            d.scenes.retain(|s| s.id != id);
            if d.current_scene == id {
                d.current_scene.clear();
            }
            store::save(&d);
            ok()
        }

        "pause_toggle" => { ctx.send(Cmd::PauseToggle); ok() }
        "open_settings" => { ctx.send(Cmd::OpenSettings); ok() }
        "quit" => { ctx.send(Cmd::Quit); ok() }
        _ => err("Unknown command."),
    }
}
