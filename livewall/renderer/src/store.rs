use crate::model::Data;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init(portable: Option<PathBuf>) {
    let dir = portable.unwrap_or_else(|| {
        PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("LiveWall")
    });
    let _ = fs::create_dir_all(dir.join("thumbs"));
    let _ = DIR.set(dir);
}

pub fn dir() -> &'static PathBuf {
    DIR.get().expect("store::init not called")
}

pub fn thumbs() -> PathBuf {
    dir().join("thumbs")
}

pub fn load() -> Data {
    let path = dir().join("state.json");
    let mut d: Data = match fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(d) => d,
            Err(_) => {
                let _ = fs::rename(&path, dir().join("state.broken.json"));
                Data::default()
            }
        },
        Err(_) => Data::default(),
    };
    d.live.id = "live".into();
    if d.live.name.is_empty() {
        d.live.name = "Current".into();
    }
    d
}

pub fn save(d: &Data) {
    let path = dir().join("state.json");
    let tmp = dir().join("state.json.tmp");
    if let Ok(s) = serde_json::to_string_pretty(d) {
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}
