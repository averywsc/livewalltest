//! Overlays: GIF, image, WebP and short looping videos drawn in click-through
//! layered windows that are children of the same WorkerW as the wallpaper, so
//! they always sit behind the desktop icons.
use crate::desktop::{self, Mon};
use crate::model::{Data, Overlay};
use crate::mpv::{Api, Mpv};
use crate::pause::Decision;
use crate::util;
use image::imageops::{resize, FilterType};
use image::{AnimationDecoder, RgbaImage};
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const MAX_DIM: u32 = 1024;
const MAX_BYTES: usize = 200 * 1024 * 1024;

pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>, // premultiplied BGRA
    pub delay: u32,
}

#[derive(Clone, PartialEq)]
struct Geom {
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    rot: f64,
    op: f64,
}

impl Geom {
    fn of(o: &Overlay) -> Geom {
        Geom { cx: o.cx, cy: o.cy, w: o.w, h: o.h, rot: o.rotation, op: o.opacity }
    }
}

struct VideoSrc {
    mpv: Mpv,
    tw: i32,
    th: i32,
    alpha: bool,
    raw: Vec<u8>,
    last: Option<Frame>,
}

impl VideoSrc {
    fn open(api: &Arc<Api>, o: &Overlay) -> Result<VideoSrc, String> {
        let alpha = util::ext(&o.path) == "webm";
        let (nw, nh) = if o.nw > 0 && o.nh > 0 { (o.nw as f64, o.nh as f64) } else { (480.0, 270.0) };
        let f = (640.0 / nw.max(nh)).min(1.0);
        let tw = ((nw * f).round() as i32).max(2) & !1;
        let th = ((nh * f).round() as i32).max(2) & !1;
        let mut opts: Vec<(&str, String)> = vec![
            ("vo", "libmpv".into()),
            ("config", "no".into()),
            ("load-scripts", "no".into()),
            ("ytdl", "no".into()),
            ("terminal", "no".into()),
            ("aid", "no".into()),
            ("loop-file", "inf".into()),
            ("hwdec", "no".into()),
            ("keepaspect", "no".into()),
            ("vd-lavc-threads", "2".into()),
            ("input-default-bindings", "no".into()),
            ("osc", "no".into()),
        ];
        if alpha {
            // Color on the left half, alpha as grey on the right half. If the decoder
            // gives no alpha plane the right half is plain white, i.e. fully opaque.
            opts.push(("vd", "lavc:libvpx-vp9".into()));
            opts.push(("lavfi-complex", "[vid1]split[c][a];[a]alphaextract[am];[c][am]hstack[vo]".into()));
        }
        let mpv = Mpv::new_sw(api, &opts)?;
        mpv.command(&["loadfile", &o.path, "replace"])?;
        let rw = if alpha { tw * 2 } else { tw };
        Ok(VideoSrc { mpv, tw, th, alpha, raw: vec![0; (rw * th * 4) as usize], last: None })
    }

    fn poll(&mut self) -> bool {
        if !self.mpv.render_has_frame() {
            return false;
        }
        let rw = if self.alpha { self.tw * 2 } else { self.tw };
        if !self.mpv.render_sw(rw, self.th, &mut self.raw) {
            return false;
        }
        let (tw, th) = (self.tw as usize, self.th as usize);
        let mut px = vec![0u8; tw * th * 4];
        for y in 0..th {
            for x in 0..tw {
                let s = (y * rw as usize + x) * 4;
                let d = (y * tw + x) * 4;
                if self.alpha {
                    let a = self.raw[(y * rw as usize + tw + x) * 4 + 2] as u32;
                    for c in 0..3 {
                        px[d + c] = ((self.raw[s + c] as u32 * a + 127) / 255) as u8;
                    }
                    px[d + 3] = a as u8;
                } else {
                    px[d..d + 3].copy_from_slice(&self.raw[s..s + 3]);
                    px[d + 3] = 255;
                }
            }
        }
        self.last = Some(Frame { w: tw as u32, h: th as u32, px, delay: 33 });
        true
    }
}

enum Src {
    Frames { frames: Vec<Frame>, idx: usize },
    Video(Box<VideoSrc>),
}

struct Ov {
    id: String,
    hwnd: HWND,
    parent: HWND,
    geom: Geom,
    src: Src,
    next_due: Instant,
    dirty: bool,
    paused: bool,
    dc: HDC,
    bmp: HBITMAP,
    bits: *mut u8,
    bw: i32,
    bh: i32,
}

pub struct Overlays {
    list: Vec<Ov>,
    pub errors: HashMap<String, String>,
}

impl Overlays {
    pub fn new() -> Overlays {
        Overlays { list: vec![], errors: HashMap::new() }
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    pub fn clear(&mut self) {
        for ov in self.list.drain(..) {
            ov.destroy();
        }
    }

    pub fn sync(&mut self, data: &Data, parent: HWND, api: &Option<Arc<Api>>) {
        let mut i = 0;
        while i < self.list.len() {
            if data.live.overlays.iter().any(|o| o.id == self.list[i].id) {
                i += 1;
            } else {
                self.list.remove(i).destroy();
            }
        }
        self.errors.retain(|id, _| data.live.overlays.iter().any(|o| &o.id == id));
        if parent.is_null() {
            return;
        }
        for o in &data.live.overlays {
            let g = Geom::of(o);
            if let Some(ex) = self.list.iter_mut().find(|e| e.id == o.id) {
                if ex.geom != g {
                    ex.geom = g;
                    ex.dirty = true;
                }
                continue;
            }
            if self.errors.contains_key(&o.id) {
                continue;
            }
            match Ov::create(o, parent, api) {
                Ok(ov) => self.list.push(ov),
                Err(e) => {
                    self.errors.insert(o.id.clone(), format!("{}: {}", o.name, e));
                }
            }
        }
        for ov in &mut self.list {
            if ov.dirty {
                ov.draw();
            }
        }
    }

    /// Keeps overlays above the wallpaper windows after those were recreated.
    pub fn raise_all(&self) {
        for ov in &self.list {
            unsafe { desktop::raise(ov.hwnd) };
        }
    }

    pub fn apply_pause(&mut self, d: &Decision, mons: &[Mon]) {
        let now = Instant::now();
        for ov in &mut self.list {
            let want = d.for_point(mons, ov.geom.cx, ov.geom.cy).is_some();
            if want != ov.paused {
                ov.paused = want;
                if let Src::Video(v) = &ov.src {
                    v.mpv.set("pause", if want { "yes" } else { "no" });
                }
                if !want {
                    ov.next_due = now;
                }
            }
        }
    }

    /// Advances animations. Returns how long until the next tick is needed, or
    /// None when nothing animates (the caller then stops its timer).
    pub fn tick(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        for ov in &mut self.list {
            if ov.paused {
                continue;
            }
            match &mut ov.src {
                Src::Frames { frames, idx } if frames.len() > 1 => {
                    if now >= ov.next_due {
                        *idx = (*idx + 1) % frames.len();
                        let d = Duration::from_millis(frames[*idx].delay.max(20) as u64);
                        let nd = ov.next_due + d;
                        ov.next_due = if nd < now { now + d } else { nd };
                        ov.dirty = true;
                    }
                }
                Src::Video(v) => {
                    if now >= ov.next_due {
                        ov.next_due = now + Duration::from_millis(33);
                        if v.poll() {
                            ov.dirty = true;
                        }
                    }
                }
                _ => continue,
            }
            if ov.dirty {
                ov.draw();
            }
            next = Some(next.map_or(ov.next_due, |n| n.min(ov.next_due)));
        }
        next.map(|t| t.saturating_duration_since(Instant::now()).max(Duration::from_millis(8)))
    }

    pub fn animated_active(&self) -> bool {
        self.list.iter().any(|o| !o.paused && match &o.src {
            Src::Frames { frames, .. } => frames.len() > 1,
            Src::Video(_) => true,
        })
    }
}

impl Ov {
    fn create(o: &Overlay, parent: HWND, api: &Option<Arc<Api>>) -> Result<Ov, String> {
        let src = if o.kind == "video" {
            let a = api.as_ref().ok_or("libmpv-2.dll was not found")?;
            Src::Video(Box::new(VideoSrc::open(a, o)?))
        } else {
            let frames = load_frames(&o.path)?;
            Src::Frames { frames, idx: 0 }
        };
        unsafe {
            let hwnd = desktop::create_child("LiveWallOverlay", parent, 0, 0, 8, 8, true);
            if hwnd.is_null() {
                return Err("could not create the overlay window".into());
            }
            Ok(Ov {
                id: o.id.clone(),
                hwnd,
                parent,
                geom: Geom::of(o),
                src,
                next_due: Instant::now(),
                dirty: true,
                paused: false,
                dc: CreateCompatibleDC(null_mut()),
                bmp: null_mut(),
                bits: null_mut(),
                bw: 0,
                bh: 0,
            })
        }
    }

    fn destroy(self) {
        unsafe {
            DestroyWindow(self.hwnd);
            if !self.bmp.is_null() {
                DeleteObject(self.bmp);
            }
            DeleteDC(self.dc);
        }
    }

    fn ensure_dib(&mut self, bw: i32, bh: i32) {
        if self.bw == bw && self.bh == bh && !self.bmp.is_null() {
            return;
        }
        unsafe {
            let mut bi: BITMAPINFO = zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = bw;
            bi.bmiHeader.biHeight = -bh;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(self.dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() {
                return;
            }
            let old = SelectObject(self.dc, bmp);
            let _ = old;
            if !self.bmp.is_null() {
                DeleteObject(self.bmp);
            }
            self.bmp = bmp;
            self.bits = bits as *mut u8;
            self.bw = bw;
            self.bh = bh;
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let g = self.geom.clone();
        let (w, h) = (g.w.max(4.0), g.h.max(4.0));
        let (s, c) = g.rot.to_radians().sin_cos();
        let bw = ((w * c.abs() + h * s.abs()).ceil() as i32).clamp(1, 8192);
        let bh = ((w * s.abs() + h * c.abs()).ceil() as i32).clamp(1, 8192);
        self.ensure_dib(bw, bh);
        if self.bits.is_null() {
            return;
        }
        let bits = self.bits;
        let frame: &Frame = match &self.src {
            Src::Frames { frames, idx } => &frames[*idx],
            Src::Video(v) => match &v.last {
                Some(f) => f,
                None => return,
            },
        };
        let dst = unsafe { std::slice::from_raw_parts_mut(bits, (bw * bh * 4) as usize) };
        blit(frame, w as f32, h as f32, g.rot as f32, g.op as f32, bw, bh, dst);
        unsafe {
            let mut pt = POINT { x: (g.cx - bw as f64 / 2.0).round() as i32, y: (g.cy - bh as f64 / 2.0).round() as i32 };
            ScreenToClient(self.parent, &mut pt);
            SetWindowPos(self.hwnd, null_mut(), pt.x, pt.y, bw, bh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let size = SIZE { cx: bw, cy: bh };
            let src_pt = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 1 };
            UpdateLayeredWindow(self.hwnd, null_mut(), null(), &size, self.dc, &src_pt, 0, &blend, 2);
        }
    }
}

/// Draws a premultiplied BGRA frame scaled to w*h, rotated by `rot` degrees
/// (clockwise) about the centre of a bw*bh destination, with opacity applied.
fn blit(f: &Frame, w: f32, h: f32, rot: f32, opacity: f32, bw: i32, bh: i32, dst: &mut [u8]) {
    let (s, c) = rot.to_radians().sin_cos();
    let (hw, hh) = (bw as f32 / 2.0, bh as f32 / 2.0);
    let (fx, fy) = (f.w as f32 / w, f.h as f32 / h);
    let op = opacity.clamp(0.0, 1.0);
    let (fw, fh) = (f.w as i32, f.h as i32);
    for y in 0..bh {
        for x in 0..bw {
            let o = ((y * bw + x) * 4) as usize;
            let dx = x as f32 + 0.5 - hw;
            let dy = y as f32 + 0.5 - hh;
            let u = dx * c + dy * s + w / 2.0;
            let v = -dx * s + dy * c + h / 2.0;
            if u < 0.0 || v < 0.0 || u >= w || v >= h {
                dst[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let sx = (u * fx - 0.5).max(0.0);
            let sy = (v * fy - 0.5).max(0.0);
            let (x0, y0) = (sx as i32, sy as i32);
            let (x1, y1) = ((x0 + 1).min(fw - 1), (y0 + 1).min(fh - 1));
            let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
            let at = |xx: i32, yy: i32, ch: usize| f.px[((yy * fw + xx) * 4) as usize + ch] as f32;
            for ch in 0..4 {
                let top = at(x0, y0, ch) * (1.0 - tx) + at(x1, y0, ch) * tx;
                let bot = at(x0, y1, ch) * (1.0 - tx) + at(x1, y1, ch) * tx;
                dst[o + ch] = ((top * (1.0 - ty) + bot * ty) * op) as u8;
            }
        }
    }
}

fn to_frame(img: RgbaImage, delay_ms: u32) -> Frame {
    let (w, h) = img.dimensions();
    let mut raw = img.into_raw();
    for p in raw.chunks_exact_mut(4) {
        let a = p[3] as u32;
        p[0] = ((p[0] as u32 * a + 127) / 255) as u8;
        p[1] = ((p[1] as u32 * a + 127) / 255) as u8;
        p[2] = ((p[2] as u32 * a + 127) / 255) as u8;
    }
    let mut img = RgbaImage::from_raw(w, h, raw).unwrap();
    if w.max(h) > MAX_DIM {
        let f = MAX_DIM as f32 / w.max(h) as f32;
        img = resize(&img, ((w as f32 * f) as u32).max(1), ((h as f32 * f) as u32).max(1), FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    let mut px = img.into_raw();
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2); // RGBA -> BGRA
    }
    Frame { w, h, px, delay: delay_ms }
}

/// Decodes GIF, WebP (animated or not) and still images. Memory is capped; a very
/// long animation is cut at the cap rather than exhausting memory.
pub fn load_frames(path: &str) -> Result<Vec<Frame>, String> {
    let ext = util::ext(path);
    let mut out: Vec<Frame> = Vec::new();
    let mut bytes = 0usize;
    let mut push = |img: RgbaImage, delay: u32, out: &mut Vec<Frame>| -> bool {
        let f = to_frame(img, delay);
        bytes += f.px.len();
        out.push(f);
        bytes < MAX_BYTES
    };
    if ext == "gif" || ext == "webp" {
        let file = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
        let frames = if ext == "gif" {
            image::codecs::gif::GifDecoder::new(file).map_err(|e| e.to_string())?.into_frames()
        } else {
            image::codecs::webp::WebPDecoder::new(file).map_err(|e| e.to_string())?.into_frames()
        };
        for fr in frames {
            let fr = fr.map_err(|e| e.to_string())?;
            let (n, d) = fr.delay().numer_denom_ms();
            let ms = if d == 0 { 100 } else { n / d };
            if !push(fr.into_buffer(), if ms == 0 { 100 } else { ms }, &mut out) {
                break;
            }
        }
    } else {
        let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
        push(img, 0, &mut out);
    }
    if out.is_empty() {
        return Err("no frames could be read".into());
    }
    Ok(out)
}
