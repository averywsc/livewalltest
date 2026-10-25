#![windows_subsystem = "windows"]
//! Portable build: one exe that carries the app, unpacks it once into
//! %LOCALAPPDATA%\LiveWallPortable\<version>, and starts it with its data kept
//! in a LiveWall-data folder next to this exe.
use std::fs;
use std::io::Cursor;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

static PAYLOAD: &[u8] = include_bytes!("../../dist/payload.zip");

fn main() {
    let me = std::env::current_exe().expect("cannot find own path");
    let home = me.parent().expect("no folder").to_path_buf();
    let base = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("LiveWallPortable");
    let dir = base.join(format!("{}-{}", env!("CARGO_PKG_VERSION"), PAYLOAD.len()));
    let exe = dir.join("livewall.exe");
    if !exe.exists() {
        let _ = fs::create_dir_all(&dir);
        let mut zip = zip::ZipArchive::new(Cursor::new(PAYLOAD)).expect("the bundled files are damaged");
        zip.extract(&dir).expect("could not unpack the bundled files");
    }
    let _ = Command::new(exe)
        .arg("--portable-dir")
        .arg(home.join("LiveWall-data"))
        .arg("--launcher")
        .arg(&me)
        .creation_flags(0x0000_0008) // DETACHED_PROCESS
        .spawn();
}
