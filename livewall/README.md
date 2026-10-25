# Live Wallpaper

A free live wallpaper app for Windows 10 and 11. Set a video, GIF or image as your wallpaper, add overlays, play music, and save it all as scenes. It runs offline. There is no account, no ads, no telemetry and no store. The library starts empty.

## Status: read this first

This code was written but **has not been compiled or run**. The environment it was written in has no Windows, no Rust toolchain and no GPU, so:

- The first build will probably show some compiler errors. They should be small (a type name or a feature flag in a Windows API call). Paste them back and they get fixed.
- The desktop attachment, two-monitor behaviour, overlay layering, WebM transparency and loop smoothness are **untested**. Section "Test checklist" tells you how to check each one.
- **No CPU, GPU or memory numbers exist yet.** `tools\measure.ps1` produces them on your machine. Nothing in this project claims a number.

## What is in it

| Folder | What it is |
|---|---|
| `renderer/` | `livewall.exe`. Rust. The background program: wallpaper windows, libmpv playback, pause rules, overlays, music, tray, control channel. |
| `settings/` | `livewall-settings.exe`. Tauri + WebView2, plain HTML/CSS/JS. A separate process. Close it and the wallpaper keeps running. |
| `launcher/` | Portable build. One exe that carries the app and unpacks it on first run. |
| `installer/` | Inno Setup script for the installer. |
| `tools/` | Build, font, icon and measuring scripts. |

Why this stack: libmpv gives Direct3D 11 hardware decoding, software fallback, and every format you listed, so there is no decoder code to maintain. Rust keeps the always-running process small. Electron is not used anywhere.

Settings and scenes are plain JSON in `%APPDATA%\LiveWall\state.json` (portable build: `LiveWall-data` next to the exe). Thumbnails are in `thumbs\`.

## Build it

You need Windows 10 or 11 x64.

1. **Install the tools** (once). In PowerShell:
   ```
   winget install Rustlang.Rustup
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   winget install JRSoftware.InnoSetup
   cargo install tauri-cli --version "^2" --locked
   ```
   Close and reopen PowerShell afterwards. Inno Setup must be 6.3 or newer. WebView2 is already on current Windows 10 and 11.
2. **Add the font** (optional, used only by the settings window): `.\tools\get-fonts.ps1`. If you skip it the window uses Segoe UI.
3. **Add libmpv.** Download an **LGPL** build of libmpv for x86_64 (for example the `mpv-dev-lgpl-x86_64-*.7z` archive from the shinchiro builds at https://github.com/shinchiro/mpv-winbuild-cmake/releases). Extract it and copy `libmpv-2.dll` into the `vendor` folder. Check the file name and that it is an LGPL build; I could not verify the current archive names from here.
4. **Build everything:**
   ```
   .\tools\build.ps1
   ```
   It builds the renderer, the settings window, the portable exe and the installer, in that order, and stops with a message if a step fails. You get:
   - `dist\LiveWallpaper-Setup.exe` (installer)
   - `dist\LiveWallpaper-Portable.exe` (portable)

To build only the two programs for quick testing:
```
cargo build --release -p livewall
cd settings
cargo tauri build --no-bundle
```
Then copy `libmpv-2.dll` next to `target\release\livewall.exe` and run `livewall.exe`.

## Use it

Run `livewall.exe` (or the installer's shortcut). A tray icon appears. Click it to open settings. Library: add a file, then "Set as wallpaper". Overlays: add a file, drag it in the layout. Music: add audio files. Settings: scenes, frame rate limit, pause rules, start with Windows.

## Test checklist

Do these in order. Each step depends on the one before.

**1. Desktop attachment (riskiest, do this first)**
- [ ] Add a looping 1080p mp4 and set it as the wallpaper. It plays behind the desktop icons, not over them.
- [ ] Desktop icons can be clicked, dragged and right-clicked. The desktop right-click menu works.
- [ ] With two monitors, a different wallpaper plays on each. Then "One across all displays" spans both.
- [ ] Repeat on both Windows 10 and Windows 11 if you have both. On Windows 11 24H2 the attachment uses a different window layout.

**2. Playback**
- [ ] mp4, mkv, webm, mov, avi, GIF and an image all show. Fill, Fit, Stretch, Center and Tile each look right.
- [ ] The loop point shows no flash or stutter. If it does, note the codec and length.
- [ ] Task Manager, GPU tab: the "Video Decode" engine shows load for an H.264 or HEVC file. If not, hardware decoding fell back to software.
- [ ] Frame rate limit 30 on a 60 fps file lowers GPU use.

**3. Pause rules**
- [ ] Open a game or a fullscreen video: the status line says Paused, and resumes instantly when you leave.
- [ ] Lock the screen (Win+L): paused. Unlock: resumes. Turn the display off by sleep timeout: paused, then resumes.
- [ ] "Pause when any window is maximized" and "Pause on battery" work when switched on.
- [ ] While paused, `tools\measure.ps1` shows close to 0 percent CPU and GPU.

**4. Multiple monitors and recovery**
- [ ] Unplug and replug a monitor, change resolution, sleep and wake: the wallpaper comes back without restarting.
- [ ] End `explorer.exe` in Task Manager and run it again: the wallpaper and tray icon come back within a few seconds.

**5. Settings window**
- [ ] Drag files onto the window and use the file picker. Rename and remove work. Close the window: the wallpaper keeps playing.
- [ ] Tab through the whole window with the keyboard. Focus outlines are visible.

**6. Overlays**
- [ ] A transparent PNG, an animated GIF and an animated WebP show with transparency. A short mp4 loops.
- [ ] Drag, resize, rotate, opacity and delete work. Overlays sit under the desktop icons and never block clicks.
- [ ] **Transparent WebM is the least certain feature.** If it shows with a black box, it means the video decoder did not give an alpha channel.

**7. Music**
- [ ] mp3, flac, wav, ogg and m4a play. Play, pause, next, previous, shuffle, loop and volume work.
- [ ] Close the settings window: music keeps playing. Video mute and music volume are independent.

**8. Scenes**
- [ ] Save a scene, change everything, load the scene: wallpaper, overlays and playlist return. Rename and delete work. Switching from the tray menu works.

**9. Tray and startup**
- [ ] Tray menu: open settings, pause, mute video, music controls, scenes, quit.
- [ ] "Start with Windows" adds the app at next sign-in, and "Restore last scene" brings the last state back.

**10. Installer and portable**
- [ ] The installer installs per user, starts the app, and the uninstaller removes it and the start-with-Windows entry.
- [ ] The portable exe runs from a USB stick or any folder and keeps its data in `LiveWall-data` beside it.

## Measure it

1. Play a 1080p loop. Wait 30 seconds, then in PowerShell: `.\tools\measure.ps1 -Process livewall -Label playing-1080p`
2. Pause with the button in the settings window, wait 10 seconds: `.\tools\measure.ps1 -Process livewall -Label paused`
3. To compare with Wallpaper Engine, play the same file there: `.\tools\measure.ps1 -Process wallpaper64,wallpaper32,webwallpaper32 -Label wallpaper-engine`

Each run prints averages and writes a per-second CSV. Report both the average and maximum.

## Known limits (stated honestly)

- **No update check.** It was optional and off by default in the brief, but it is a network call, so it is left out entirely. The app makes no network connections.
- **Transparent WebM** relies on an alpha-capable VP9 decoder inside your libmpv build. If it is missing, WebM overlays show opaque.
- **Video overlays use the processor** (software rendering at up to 640 px). Keep them short and small. GIF, WebP and image overlays are cheap.
- **Tile mode** on a video repeats frames on the processor. It is intended for images and GIFs.
- **A frame rate limit below the video's own rate** switches to copied (not zero-copy) hardware decoding, because mpv's frame-rate filter cannot take GPU frames.
- **Overlay positions are in screen pixels.** If you change monitor layout, an overlay may end up off-screen; drag it back in the layout view or delete it.
- Windows 10 and 11, x64 only.
- The installer is per-user by default, so it does not need administrator rights.

## Licence

MIT, see `LICENSE`. Third-party components: see `THIRD_PARTY.md`. The libmpv DLL must be an LGPL build to keep the whole app MIT.
