# Third-party software

Direct dependencies only. Run `cargo install cargo-about` then `cargo about generate about.hbs`
(or `cargo install cargo-license && cargo license`) before a release to produce the full list of
transitive crates, mainly those pulled in by Tauri.

| Component | Used for | Licence |
|---|---|---|
| libmpv (LGPL build, loaded as libmpv-2.dll) | Video decoding and rendering | LGPL-2.1-or-later. Linked dynamically; you can replace the DLL with another LGPL build. A GPL build of libmpv would make the combined work GPL. |
| FFmpeg (inside libmpv) | Codecs, filters | LGPL-2.1-or-later in LGPL builds |
| libloading | Loading libmpv-2.dll | ISC |
| serde, serde_json | JSON files and control messages | MIT OR Apache-2.0 |
| rodio | Audio output | MIT OR Apache-2.0 |
| symphonia (via rodio) | mp3, flac, wav, ogg, m4a decoding | MPL-2.0 |
| image (with png, gif, jpeg, webp, bmp codecs) | Thumbnails, GIF/WebP/image overlays | MIT OR Apache-2.0 |
| windows-sys | Win32 API bindings | MIT OR Apache-2.0 |
| winres | Embedding icon and manifest | MIT |
| Tauri 2 | Settings window | MIT OR Apache-2.0 |
| tauri-plugin-dialog | File picker | MIT OR Apache-2.0 |
| base64 | Thumbnail data URLs | MIT OR Apache-2.0 |
| zip | Portable launcher | MIT |
| Microsoft Edge WebView2 Runtime | Renders the settings window; already part of Windows 11 and current Windows 10 | Microsoft Evergreen runtime licence (not bundled) |
| IBM Plex Sans (optional, downloaded at build time) | Typeface | SIL Open Font License 1.1 |
| Inno Setup (build tool only) | Installer | Inno Setup License (not shipped) |
