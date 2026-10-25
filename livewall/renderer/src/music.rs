//! Music player on its own thread. It owns the audio device, so music keeps
//! playing when the settings window is closed. While stopped or paused the
//! thread sleeps in a blocking receive and does no work.
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use serde::Serialize;
use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Default, Serialize)]
pub struct MusicStatus {
    pub playing: bool,
    pub paused: bool,
    pub index: usize,
    pub count: usize,
    pub error: String,
}

pub enum MCmd {
    Load { paths: Vec<String>, index: usize, autoplay: bool, volume: u32, shuffle: bool, loop_mode: String },
    SetList(Vec<String>),
    Play,
    Pause,
    Toggle,
    Next,
    Prev,
    Select(usize),
    Volume(u32),
    Shuffle(bool),
    LoopMode(String),
    Quit,
}

pub fn spawn() -> (Sender<MCmd>, Arc<Mutex<MusicStatus>>) {
    let (tx, rx) = mpsc::channel();
    let st = Arc::new(Mutex::new(MusicStatus::default()));
    let st2 = st.clone();
    std::thread::spawn(move || {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
        let mut p = Player {
            rx, st: st2, out: None, sink: None, tracks: vec![], idx: 0, vol: 0.25, shuffle: false,
            loop_mode: "all".into(), playing: false, paused: false, hist: vec![], rng: seed,
        };
        p.run();
    });
    (tx, st)
}

struct Player {
    rx: Receiver<MCmd>,
    st: Arc<Mutex<MusicStatus>>,
    out: Option<(OutputStream, OutputStreamHandle)>,
    sink: Option<Sink>,
    tracks: Vec<String>,
    idx: usize,
    vol: f32,
    shuffle: bool,
    loop_mode: String,
    playing: bool,
    paused: bool,
    hist: Vec<usize>,
    rng: u64,
}

impl Player {
    fn run(&mut self) {
        loop {
            let msg = if self.playing && !self.paused {
                self.rx.recv_timeout(Duration::from_millis(500))
            } else {
                self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match msg {
                Ok(MCmd::Quit) => {
                    self.stop();
                    break;
                }
                Ok(c) => self.handle(c),
                Err(RecvTimeoutError::Timeout) => {
                    if self.sink.as_ref().map_or(false, |s| s.empty()) {
                        self.advance(true);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn handle(&mut self, c: MCmd) {
        match c {
            MCmd::Load { paths, index, autoplay, volume, shuffle, loop_mode } => {
                self.stop();
                self.tracks = paths;
                self.idx = index.min(self.tracks.len().saturating_sub(1));
                self.vol = Self::curve(volume);
                self.shuffle = shuffle;
                self.loop_mode = loop_mode;
                self.hist.clear();
                if autoplay && !self.tracks.is_empty() {
                    self.start();
                }
            }
            MCmd::SetList(paths) => {
                let current = self.tracks.get(self.idx).cloned();
                self.tracks = paths;
                match current.and_then(|c| self.tracks.iter().position(|t| *t == c)) {
                    Some(i) => self.idx = i,
                    None => {
                        let was_playing = self.playing && !self.paused;
                        self.idx = self.idx.min(self.tracks.len().saturating_sub(1));
                        self.stop();
                        if was_playing && !self.tracks.is_empty() {
                            self.start();
                        }
                    }
                }
            }
            MCmd::Play => {
                if self.playing && self.paused {
                    if let Some(s) = &self.sink {
                        s.play();
                    }
                    self.paused = false;
                } else if !self.playing && !self.tracks.is_empty() {
                    self.start();
                }
            }
            MCmd::Pause => {
                if self.playing && !self.paused {
                    if let Some(s) = &self.sink {
                        s.pause();
                    }
                    self.paused = true;
                }
            }
            MCmd::Toggle => {
                if self.playing && !self.paused {
                    self.handle(MCmd::Pause);
                } else {
                    self.handle(MCmd::Play);
                }
            }
            MCmd::Next => {
                if !self.tracks.is_empty() {
                    self.advance(false);
                }
            }
            MCmd::Prev => {
                if let Some(i) = self.hist.pop() {
                    self.idx = i;
                } else if !self.tracks.is_empty() {
                    self.idx = (self.idx + self.tracks.len() - 1) % self.tracks.len();
                }
                if !self.tracks.is_empty() {
                    self.start();
                }
            }
            MCmd::Select(i) => {
                if i < self.tracks.len() {
                    self.hist.push(self.idx);
                    self.idx = i;
                    self.start();
                }
            }
            MCmd::Volume(v) => {
                self.vol = Self::curve(v);
                if let Some(s) = &self.sink {
                    s.set_volume(self.vol);
                }
            }
            MCmd::Shuffle(b) => self.shuffle = b,
            MCmd::LoopMode(m) => self.loop_mode = m,
            MCmd::Quit => {}
        }
        self.publish("");
    }

    fn curve(v: u32) -> f32 {
        let x = (v.min(100) as f32) / 100.0;
        x * x
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn publish(&self, err: &str) {
        if let Ok(mut s) = self.st.lock() {
            s.playing = self.playing;
            s.paused = self.paused;
            s.index = self.idx;
            s.count = self.tracks.len();
            if !err.is_empty() || !self.playing {
                s.error = err.to_string();
            }
        }
    }

    fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.out = None;
        self.playing = false;
        self.paused = false;
    }

    /// Starts the track at self.idx; skips unreadable files (at most one full pass).
    fn start(&mut self) {
        self.stop();
        let n = self.tracks.len();
        let mut last_err = String::new();
        for _ in 0..n {
            let path = self.tracks[self.idx].clone();
            match self.open(&path) {
                Ok(()) => {
                    self.playing = true;
                    self.publish("");
                    return;
                }
                Err(e) => {
                    last_err = format!("{path}: {e}");
                    self.idx = (self.idx + 1) % n;
                }
            }
        }
        self.publish(&last_err);
    }

    fn open(&mut self, path: &str) -> Result<(), String> {
        let (stream, handle) = OutputStream::try_default().map_err(|e| format!("no audio device ({e})"))?;
        let sink = Sink::try_new(&handle).map_err(|e| e.to_string())?;
        let file = File::open(path).map_err(|e| e.to_string())?;
        let dec = Decoder::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        sink.set_volume(self.vol);
        sink.append(dec);
        self.out = Some((stream, handle));
        self.sink = Some(sink);
        Ok(())
    }

    fn advance(&mut self, auto: bool) {
        let n = self.tracks.len();
        if n == 0 {
            self.stop();
            self.publish("");
            return;
        }
        let next = if auto && self.loop_mode == "one" {
            Some(self.idx)
        } else if self.shuffle && n > 1 {
            let mut i = (self.rand() % n as u64) as usize;
            if i == self.idx {
                i = (i + 1) % n;
            }
            Some(i)
        } else if self.idx + 1 < n {
            Some(self.idx + 1)
        } else if !auto || self.loop_mode == "all" {
            Some(0)
        } else {
            None
        };
        match next {
            Some(i) => {
                if i != self.idx {
                    self.hist.push(self.idx);
                    if self.hist.len() > 200 {
                        self.hist.remove(0);
                    }
                }
                self.idx = i;
                self.start();
            }
            None => {
                self.stop();
                self.publish("");
            }
        }
    }
}
