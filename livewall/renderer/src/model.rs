use serde::{Deserialize, Serialize};

fn fill() -> String { "fill".into() }
fn one() -> f64 { 1.0 }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Item {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: String, // video | gif | image
    #[serde(default)] pub thumb: String,
    #[serde(default)] pub fps: f64,
    #[serde(default)] pub duration: f64,
    #[serde(default)] pub width: u32,
    #[serde(default)] pub height: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonitorAssign {
    pub device: String,
    pub item: String,
    #[serde(default = "fill")] pub fit: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Wallpaper {
    pub span: bool,
    pub span_item: String,
    pub span_fit: String,
    pub monitors: Vec<MonitorAssign>,
}
impl Default for Wallpaper {
    fn default() -> Self {
        Wallpaper { span: false, span_item: String::new(), span_fit: fill(), monitors: vec![] }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Overlay {
    pub id: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    #[serde(default)] pub thumb: String,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)] pub rotation: f64,
    #[serde(default = "one")] pub opacity: f64,
    #[serde(default)] pub nw: u32,
    #[serde(default)] pub nh: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Track {
    pub path: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct MusicConf {
    pub playlist: Vec<Track>,
    pub current: usize,
    pub volume: u32,
    pub shuffle: bool,
    pub loop_mode: String, // off | all | one
    pub autoplay: bool,
}
impl Default for MusicConf {
    fn default() -> Self {
        MusicConf { playlist: vec![], current: 0, volume: 50, shuffle: false, loop_mode: "all".into(), autoplay: false }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Scene {
    pub id: String,
    pub name: String,
    pub wallpaper: Wallpaper,
    pub overlays: Vec<Overlay>,
    pub music: MusicConf,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub fps_cap: u32, // 0 = match the video
    pub pause_fullscreen: bool,
    pub pause_maximized: bool,
    pub pause_battery: bool,
    pub video_muted: bool,
    pub video_volume: u32,
    pub start_with_windows: bool,
    pub restore_last: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Settings {
            fps_cap: 0,
            pause_fullscreen: true,
            pause_maximized: false,
            pause_battery: false,
            video_muted: true,
            video_volume: 50,
            start_with_windows: false,
            restore_last: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Data {
    pub library: Vec<Item>,
    pub settings: Settings,
    pub live: Scene,
    pub scenes: Vec<Scene>,
    pub current_scene: String,
}

impl Data {
    pub fn item(&self, id: &str) -> Option<&Item> {
        self.library.iter().find(|i| i.id == id)
    }

    /// Copies a saved scene into the live scene. Returns false if the id is unknown.
    pub fn load_scene(&mut self, id: &str) -> bool {
        match self.scenes.iter().find(|s| s.id == id).cloned() {
            Some(mut s) => {
                s.id = "live".into();
                self.live = s;
                self.current_scene = id.to_string();
                true
            }
            None => false,
        }
    }
}

#[derive(Serialize, Clone, Default)]
pub struct MonJson {
    pub device: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub primary: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct Runtime {
    pub paused: String,       // empty when playing
    pub manual_pause: bool,
    pub errors: Vec<String>,
    pub mpv_ok: bool,
    pub active: bool,         // a wallpaper or overlay is on screen
    pub monitors: Vec<MonJson>,
}
