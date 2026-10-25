#![windows_subsystem = "windows"]

mod autostart;
mod desktop;
mod engine;
mod ipc;
mod model;
mod mpv;
mod music;
mod overlay;
mod pause;
mod store;
mod thumb;
mod tray;
mod util;

use ipc::Cmd;
use model::*;
use music::{MCmd, MusicStatus};
use std::cell::RefCell;
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use util::wide;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows_sys::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub static CTRL: AtomicIsize = AtomicIsize::new(0);
pub const WM_APP_CMD: u32 = 0x8000 + 1;
const WM_APP_TRAY: u32 = 0x8000 + 2;
const WM_APP_MPV: u32 = 0x8000 + 3;
const WM_APP_EVAL: u32 = 0x8000 + 4;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

const T_EVAL: usize = 1;
const T_SAFETY: usize = 2;
const T_REBUILD: usize = 3;
const T_OVL: usize = 4;

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
}

struct App {
    hwnd: HWND,
    data: Arc<Mutex<Data>>,
    rt: Arc<Mutex<Runtime>>,
    rx: Receiver<Cmd>,
    api: Option<Arc<mpv::Api>>,
    api_error: String,
    engine: engine::Engine,
    overlays: overlay::Overlays,
    music_tx: Sender<MCmd>,
    music_st: Arc<Mutex<MusicStatus>>,
    flags: pause::Flags,
    applied: Settings,
    fg_hooks: Vec<isize>,
    loc_hooks: Vec<isize>,
    eval_pending: bool,
    tb_msg: u32,
    retries: u32,
    reason: String,
    launcher: Option<String>,
    child_args: Vec<String>,
    rebuild_times: Vec<Instant>,
}

unsafe extern "C" fn mpv_wake(_: *mut c_void) {
    let h = CTRL.load(Ordering::Relaxed);
    if h != 0 {
        PostMessageW(h as HWND, WM_APP_MPV, 0, 0);
    }
}

unsafe extern "system" fn win_event(_h: *mut c_void, _event: u32, _hwnd: HWND, idobj: i32, idchild: i32, _tid: u32, _t: u32) {
    if idobj != 0 || idchild != 0 {
        return;
    }
    let h = CTRL.load(Ordering::Relaxed);
    if h != 0 {
        PostMessageW(h as HWND, WM_APP_EVAL, 0, 0);
    }
}

impl App {
    fn data_clone(&self) -> Data {
        self.data.lock().unwrap().clone()
    }

    fn settings(&self) -> Settings {
        self.data.lock().unwrap().settings.clone()
    }

    fn timer(&self, id: usize, ms: u32) {
        unsafe { SetTimer(self.hwnd, id, ms.max(10), None) };
    }

    fn kill(&self, id: usize) {
        unsafe { KillTimer(self.hwnd, id) };
    }

    fn ensure_parent(&mut self) -> HWND {
        unsafe {
            if self.engine.parent.is_null() || IsWindow(self.engine.parent) == 0 {
                self.engine.parent = desktop::find_workerw();
            }
        }
        self.engine.parent
    }

    /// Recreates every wallpaper surface and overlay from the live scene.
    fn rebuild_all(&mut self) {
        let data = self.data_clone();
        let mut ready = self.engine.rebuild(&data);
        self.overlays.clear();
        self.overlays.errors.clear();
        if !data.live.overlays.is_empty() {
            let parent = self.ensure_parent();
            if parent.is_null() {
                ready = false;
            } else {
                self.overlays.sync(&data, parent, &self.api);
                self.overlays.raise_all();
            }
        }
        if !ready && self.retries < 30 {
            self.retries += 1;
            self.timer(T_REBUILD, 2000);
        } else if ready {
            self.retries = 0;
        }
        self.after_change();
    }

    fn rebuild_surfaces(&mut self) {
        let data = self.data_clone();
        if !self.engine.rebuild(&data) && self.retries < 30 {
            self.retries += 1;
            self.timer(T_REBUILD, 2000);
        }
        self.overlays.raise_all();
        self.after_change();
    }

    fn after_change(&mut self) {
        self.eval();
        self.update_hooks();
    }

    fn sync_music(&self) {
        let d = self.data.lock().unwrap();
        let m = &d.live.music;
        let _ = self.music_tx.send(MCmd::Load {
            paths: m.playlist.iter().map(|t| t.path.clone()).collect(),
            index: m.current,
            autoplay: m.autoplay,
            volume: m.volume,
            shuffle: m.shuffle,
            loop_mode: m.loop_mode.clone(),
        });
    }

    fn apply_scene(&mut self) {
        self.rebuild_all();
        self.sync_music();
    }

    fn eval(&mut self) {
        let settings = self.settings();
        let mons = desktop::monitors();
        let d = pause::evaluate(&settings, &self.flags);
        self.engine.apply_pause(&d, &mons);
        self.overlays.apply_pause(&d, &mons);
        self.reason = String::new();
        if !self.engine.surfaces.is_empty() {
            let reasons: Vec<Option<&str>> = self
                .engine
                .surfaces
                .iter()
                .map(|s| if s.span { d.for_span(&mons) } else { d.for_monitor(&s.device) })
                .collect();
            if reasons.iter().all(|r| r.is_some()) {
                self.reason = reasons[0].unwrap_or("").to_string();
            }
        } else if let Some(g) = d.global {
            self.reason = g.to_string();
        }
        if self.overlays.animated_active() {
            self.timer(T_OVL, 10);
        } else {
            self.kill(T_OVL);
        }
        self.refresh_rt();
    }

    /// Window hooks only exist while something is on screen and a rule needs them.
    fn update_hooks(&mut self) {
        let s = self.settings();
        let active = !self.engine.surfaces.is_empty() || self.overlays.count() > 0;
        let need_fg = active && (s.pause_fullscreen || s.pause_maximized);
        let need_loc = active && s.pause_maximized;
        unsafe {
            if need_fg && self.fg_hooks.is_empty() {
                for (lo, hi) in [(0x0003u32, 0x0003u32), (0x0016, 0x0017), (0x000B, 0x000B)] {
                    let h = SetWinEventHook(lo, hi, null_mut(), Some(win_event), 0, 0, 2);
                    if !h.is_null() {
                        self.fg_hooks.push(h as isize);
                    }
                }
            } else if !need_fg {
                for h in self.fg_hooks.drain(..) {
                    UnhookWinEvent(h as *mut c_void);
                }
            }
            if need_loc && self.loc_hooks.is_empty() {
                let h = SetWinEventHook(0x800B, 0x800B, null_mut(), Some(win_event), 0, 0, 2);
                if !h.is_null() {
                    self.loc_hooks.push(h as isize);
                }
            } else if !need_loc {
                for h in self.loc_hooks.drain(..) {
                    UnhookWinEvent(h as *mut c_void);
                }
            }
        }
        if active {
            self.timer(T_SAFETY, 5000);
        } else {
            self.kill(T_SAFETY);
        }
    }

    fn refresh_rt(&self) {
        let mons = desktop::monitors();
        let mut rt = self.rt.lock().unwrap();
        rt.paused = self.reason.clone();
        rt.manual_pause = self.flags.manual;
        rt.mpv_ok = self.api.is_some();
        rt.active = !self.engine.surfaces.is_empty() || self.overlays.count() > 0;
        let mut errs = self.engine.errors();
        if self.api.is_none() {
            errs.push(self.api_error.clone());
        }
        errs.extend(self.overlays.errors.values().cloned());
        rt.errors = errs;
        rt.monitors = mons
            .iter()
            .map(|m| {
                let digits: String = m.device.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
                MonJson {
                    device: m.device.clone(),
                    label: format!("Display {}", if digits.is_empty() { "?".into() } else { digits }),
                    x: m.x, y: m.y, w: m.w, h: m.h, primary: m.primary,
                }
            })
            .collect();
    }

    fn autostart_command(&self) -> String {
        match &self.launcher {
            Some(l) => format!("\"{l}\""),
            None => format!("\"{}\" --autostart", util::exe_path().display()),
        }
    }

    fn open_settings(&self) {
        let exe = util::exe_dir().join("livewall-settings.exe");
        let _ = std::process::Command::new(exe).args(&self.child_args).spawn();
    }

    fn quit(&mut self) {
        let st = self.music_st.lock().unwrap().clone();
        {
            let mut d = self.data.lock().unwrap();
            d.live.music.current = st.index;
            d.live.music.autoplay = st.playing && !st.paused;
            store::save(&d);
        }
        let _ = self.music_tx.send(MCmd::Quit);
        unsafe { tray::remove(self.hwnd) };
        for h in self.fg_hooks.drain(..).chain(self.loc_hooks.drain(..)) {
            unsafe { UnhookWinEvent(h as *mut c_void) };
        }
        self.engine.destroy_all();
        self.overlays.clear();
        unsafe { PostQuitMessage(0) };
    }

    fn drain(&mut self) {
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Wallpaper => {
                    self.retries = 0;
                    self.rebuild_surfaces();
                }
                Cmd::Overlays => {
                    let data = self.data_clone();
                    let parent = self.ensure_parent();
                    self.overlays.sync(&data, parent, &self.api);
                    self.after_change();
                }
                Cmd::Settings => {
                    let new = self.settings();
                    let old = self.applied.clone();
                    if new.fps_cap != old.fps_cap {
                        self.applied = new.clone();
                        self.rebuild_surfaces();
                    } else if new.video_muted != old.video_muted || new.video_volume != old.video_volume {
                        self.engine.set_audio(&new);
                    }
                    if new.start_with_windows != old.start_with_windows {
                        autostart::set(new.start_with_windows, &self.autostart_command());
                    }
                    self.applied = new;
                    self.after_change();
                }
                Cmd::Scene => {
                    self.retries = 0;
                    self.apply_scene();
                }
                Cmd::PauseToggle => {
                    self.flags.manual = !self.flags.manual;
                    self.eval();
                }
                Cmd::OpenSettings => self.open_settings(),
                Cmd::Quit => self.quit(),
            }
        }
    }

    fn menu_model(&self) -> tray::MenuModel {
        let d = self.data.lock().unwrap();
        let st = self.music_st.lock().unwrap();
        tray::MenuModel {
            paused_by_user: self.flags.manual,
            muted: d.settings.video_muted,
            music_playing: st.playing && !st.paused,
            scenes: d.scenes.iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
            current_scene: d.current_scene.clone(),
        }
    }

    fn on_menu(&mut self, cmd: u32) {
        match cmd {
            tray::ID_OPEN => self.open_settings(),
            tray::ID_PAUSE => {
                self.flags.manual = !self.flags.manual;
                self.eval();
            }
            tray::ID_MUTE => {
                {
                    let mut d = self.data.lock().unwrap();
                    d.settings.video_muted = !d.settings.video_muted;
                    store::save(&d);
                }
                let s = self.settings();
                self.engine.set_audio(&s);
                self.applied = s;
            }
            tray::ID_MUSIC_TOGGLE => {
                let _ = self.music_tx.send(MCmd::Toggle);
            }
            tray::ID_MUSIC_NEXT => {
                let _ = self.music_tx.send(MCmd::Next);
            }
            tray::ID_MUSIC_PREV => {
                let _ = self.music_tx.send(MCmd::Prev);
            }
            tray::ID_QUIT => self.quit(),
            c if c >= tray::ID_SCENE_BASE && c < tray::ID_QUIT => {
                let id = {
                    let d = self.data.lock().unwrap();
                    d.scenes.get((c - tray::ID_SCENE_BASE) as usize).map(|s| s.id.clone())
                };
                if let Some(id) = id {
                    {
                        let mut d = self.data.lock().unwrap();
                        d.load_scene(&id);
                        store::save(&d);
                    }
                    self.retries = 0;
                    self.apply_scene();
                }
            }
            _ => {}
        }
    }

    fn note_rebuild(&mut self) -> bool {
        let now = Instant::now();
        self.rebuild_times.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        self.rebuild_times.push(now);
        self.rebuild_times.len() <= 6
    }

    fn on_message(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_APP_CMD => {
                self.drain();
                Some(0)
            }
            WM_APP_EVAL => {
                if !self.eval_pending {
                    self.eval_pending = true;
                    self.timer(T_EVAL, 150);
                }
                Some(0)
            }
            WM_APP_MPV => {
                if self.engine.drain_events() {
                    if self.note_rebuild() {
                        self.timer(T_REBUILD, 1500);
                    } else {
                        self.engine.last_error = "The video player keeps stopping. Try another file.".into();
                    }
                }
                self.refresh_rt();
                Some(0)
            }
            WM_APP_TRAY => {
                if (lp as u32 & 0xFFFF) == WM_LBUTTONUP {
                    self.open_settings();
                }
                Some(0)
            }
            WM_TIMER => {
                match wp {
                    T_EVAL => {
                        self.kill(T_EVAL);
                        self.eval_pending = false;
                        self.eval();
                    }
                    T_SAFETY => self.eval(),
                    T_REBUILD => {
                        self.kill(T_REBUILD);
                        self.rebuild_all();
                    }
                    T_OVL => match self.overlays.tick() {
                        Some(d) => self.timer(T_OVL, d.as_millis() as u32),
                        None => self.kill(T_OVL),
                    },
                    _ => {}
                }
                Some(0)
            }
            WM_DISPLAYCHANGE => {
                self.retries = 0;
                self.timer(T_REBUILD, 1500);
                Some(0)
            }
            WM_POWERBROADCAST => {
                match wp as u32 {
                    4 => {
                        self.flags.asleep = true;
                        self.eval();
                    }
                    7 | 0x12 => {
                        self.flags.asleep = false;
                        self.retries = 0;
                        self.timer(T_REBUILD, 3000);
                    }
                    0xA => self.eval(),
                    0x8013 => unsafe {
                        let ps = &*(lp as *const POWERBROADCAST_SETTING);
                        if ps.PowerSetting.data1 == 0x6fe69556 && ps.DataLength >= 4 {
                            let v = std::ptr::read_unaligned(ps.Data.as_ptr() as *const u32);
                            self.flags.display_off = v == 0;
                            self.eval();
                        }
                    },
                    _ => {}
                }
                Some(1)
            }
            WM_WTSSESSION_CHANGE => {
                match wp {
                    7 => self.flags.locked = true,
                    8 => self.flags.locked = false,
                    _ => {}
                }
                self.eval();
                Some(0)
            }
            m if m == self.tb_msg => {
                unsafe { tray::add(hwnd, WM_APP_TRAY) };
                self.retries = 0;
                self.timer(T_REBUILD, 2000);
                Some(0)
            }
            _ => None,
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // The tray menu runs its own message loop, so it must run without the App borrowed.
    if msg == WM_APP_TRAY && (lp as u32 & 0xFFFF) == WM_RBUTTONUP {
        let model = APP.with(|c| c.try_borrow().ok().and_then(|g| g.as_ref().map(|a| a.menu_model())));
        if let Some(m) = model {
            let cmd = tray::show_menu(hwnd, &m);
            if cmd != 0 {
                APP.with(|c| {
                    if let Ok(mut g) = c.try_borrow_mut() {
                        if let Some(a) = g.as_mut() {
                            a.on_menu(cmd);
                        }
                    }
                });
            }
        }
        return 0;
    }
    let handled = APP.with(|c| {
        let Ok(mut g) = c.try_borrow_mut() else { return None };
        let a = g.as_mut()?;
        a.on_message(hwnd, msg, wp, lp)
    });
    match handled {
        Some(r) => r,
        None => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn ask_running_instance_to_open_settings() {
    for _ in 0..10 {
        if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open(ipc::PIPE_NAME) {
            let _ = f.write_all(b"{\"cmd\":\"open_settings\"}\n");
            let mut line = String::new();
            let _ = BufReader::new(&f).read_line(&mut line);
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn main() {
    // --portable-dir <dir> --launcher <exe>; every argument is also handed to the settings window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut portable: Option<PathBuf> = None;
    let mut launcher: Option<String> = None;
    let mut child_args: Vec<String> = vec![];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--portable-dir" if i + 1 < args.len() => {
                portable = Some(PathBuf::from(&args[i + 1]));
                child_args.extend([args[i].clone(), args[i + 1].clone()]);
                i += 1;
            }
            "--launcher" if i + 1 < args.len() => {
                launcher = Some(args[i + 1].clone());
                child_args.extend([args[i].clone(), args[i + 1].clone()]);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    unsafe {
        let _mutex = CreateMutexW(null(), 0, wide("LiveWallDaemon").as_ptr());
        if GetLastError() == 183 {
            ask_running_instance_to_open_settings();
            return;
        }

        store::init(portable);
        let mut data = store::load();
        data.live.id = "live".into();
        let data = Arc::new(Mutex::new(data));
        let rt = Arc::new(Mutex::new(Runtime::default()));

        let (api, api_error) = match mpv::Api::load() {
            Ok(a) => (Some(a), String::new()),
            Err(e) => (None, e),
        };

        let hinst = GetModuleHandleW(null());
        let class = wide("LiveWallCtl");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("Live Wallpaper control").as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        if hwnd.is_null() {
            return;
        }
        CTRL.store(hwnd as isize, Ordering::Relaxed);
        desktop::register_classes();

        let (music_tx, music_st) = music::spawn();
        let (tx, rx) = mpsc::channel();
        ipc::spawn(ipc::Ctx {
            data: data.clone(),
            rt: rt.clone(),
            tx,
            api: api.clone(),
            music: music_tx.clone(),
            music_st: music_st.clone(),
        });

        WTSRegisterSessionNotification(hwnd, 0);
        let display_state = GUID { data1: 0x6fe69556, data2: 0x704a, data3: 0x47a0, data4: [0x8f, 0x24, 0xc2, 0x8d, 0x93, 0x6f, 0xda, 0x47] };
        RegisterPowerSettingNotification(hwnd, &display_state, 0);
        let tb_msg = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        tray::add(hwnd, WM_APP_TRAY);

        let settings = data.lock().unwrap().settings.clone();
        let app = App {
            hwnd,
            data: data.clone(),
            rt,
            rx,
            api: api.clone(),
            api_error,
            engine: engine::Engine::new(api, mpv_wake),
            overlays: overlay::Overlays::new(),
            music_tx,
            music_st,
            flags: pause::Flags::default(),
            applied: settings.clone(),
            fg_hooks: vec![],
            loc_hooks: vec![],
            eval_pending: false,
            tb_msg,
            retries: 0,
            reason: String::new(),
            launcher,
            child_args,
            rebuild_times: vec![],
        };
        autostart::set(settings.start_with_windows, &app.autostart_command());
        APP.with(|c| *c.borrow_mut() = Some(app));
        APP.with(|c| {
            if let Some(a) = c.borrow_mut().as_mut() {
                if settings.restore_last {
                    a.apply_scene();
                } else {
                    a.refresh_rt();
                }
            }
        });

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    std::process::exit(0);
}
