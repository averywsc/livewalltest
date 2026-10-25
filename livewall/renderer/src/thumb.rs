//! Thumbnails and metadata for library items and overlays.
use crate::mpv::*;
use crate::{store, util};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Probe {
    pub thumb: String,
    pub fps: f64,
    pub duration: f64,
    pub w: u32,
    pub h: u32,
}

pub fn make(api: Option<&Arc<Api>>, path: &str, kind: &str, id: &str) -> Probe {
    if kind == "video" {
        return match api {
            Some(a) => probe_video(a, path, id),
            None => Probe::default(),
        };
    }
    let (w, h) = image::image_dimensions(path).unwrap_or((0, 0));
    let mut thumb = String::new();
    if let Ok(img) = image::open(path) {
        let out = store::thumbs().join(format!("{id}.png"));
        if img.thumbnail(360, 360).save(&out).is_ok() {
            thumb = out.to_string_lossy().to_string();
        }
    }
    Probe { thumb, w, h, ..Default::default() }
}

fn probe_video(api: &Arc<Api>, path: &str, id: &str) -> Probe {
    let tmp = store::thumbs().join(format!("{id}.tmp"));
    let _ = fs::remove_dir_all(&tmp);
    let _ = fs::create_dir_all(&tmp);
    let opts: Vec<(&str, String)> = vec![
        ("vo", "image".into()),
        ("vo-image-format", "jpg".into()),
        ("vo-image-outdir", tmp.to_string_lossy().to_string()),
        ("frames", "1".into()),
        ("start", "10%".into()),
        ("aid", "no".into()),
        ("config", "no".into()),
        ("load-scripts", "no".into()),
        ("ytdl", "no".into()),
        ("terminal", "no".into()),
        ("hwdec", "no".into()),
        ("vf", "scale=480:-2".into()),
        ("input-default-bindings", "no".into()),
    ];
    let mut pr = Probe::default();
    if let Ok(m) = Mpv::new(api, &opts) {
        if m.command(&["loadfile", path, "replace"]).is_ok() {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                let ev = m.wait_event(1.0);
                match ev.id {
                    EV_FILE_LOADED | EV_VIDEO_RECONFIG | EV_PLAYBACK_RESTART => read_meta(&m, &mut pr),
                    EV_END_FILE | EV_SHUTDOWN => break,
                    _ => {}
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(&tmp) {
        for e in rd.flatten() {
            if util::ext(&e.path().to_string_lossy()) == "jpg" {
                let out = store::thumbs().join(format!("{id}.jpg"));
                if fs::rename(e.path(), &out).is_ok() {
                    pr.thumb = out.to_string_lossy().to_string();
                }
                break;
            }
        }
    }
    let _ = fs::remove_dir_all(&tmp);
    pr
}

fn read_meta(m: &Mpv, pr: &mut Probe) {
    if pr.fps <= 0.0 {
        pr.fps = m.get("container-fps").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    }
    if pr.duration <= 0.0 {
        pr.duration = m.get("duration").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    }
    if pr.w == 0 {
        let n: usize = m.get("track-list/count").and_then(|s| s.parse().ok()).unwrap_or(0);
        for i in 0..n {
            if m.get(&format!("track-list/{i}/type")).as_deref() == Some("video") {
                pr.w = m.get(&format!("track-list/{i}/demux-w")).and_then(|s| s.parse().ok()).unwrap_or(0);
                pr.h = m.get(&format!("track-list/{i}/demux-h")).and_then(|s| s.parse().ok()).unwrap_or(0);
                break;
            }
        }
    }
}
