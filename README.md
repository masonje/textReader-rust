# Clipboard Reader (Rust version, preview)

A rewrite of Clipboard Reader in Rust. It's a single program with no Python, `.venv` or PyInstaller. The Python app in the main folder is unchanged and is still the main version. This one is being built in stages until it does everything the Python app does.

It uses the same `~/.textReader/settings.json`, the same downloaded Piper voices, and the same Edge voice list as the Python app. When it saves a setting, it changes only that key and leaves the Python app's other settings alone.

## Status: stage 2 in progress

| | |
|---|---|
| ✅ Engines | Piper (offline, same voices and downloads), Edge TTS, gTTS, eSpeak-NG |
| ✅ Playback | mpv, with speed changes while reading; Play, Pause, Stop |
| ✅ Window | the Compact view: Engine, Voice, Speed, Read Clipboard, Play/Pause, Stop, progress and time, hover hints with each shortcut |
| ✅ Shortcuts | the same four global shortcuts, read from your settings |
| ✅ Tray icon | with Read Clipboard, Stop and Quit |
| ✅ Locks | Engine and Voice are locked while reading |
| ✅ Options | Engines & Voices (on/off, Preview, download/delete, measured speed), Pronunciations, Links/paths/codes with Try it, Shortcuts |
| ✅ Text | pronunciation rules and the links/paths/codes filters apply to every read, checked against 397 cases from the Python version |
| ⏳ Stage 2 | the Full view with word highlighting, right-click a word to add a rule, Surprise me |
| ⏳ Stage 3 | Festival, system voices (pyttsx3), Coqui, Windows and macOS builds |

## Building and running

You need Rust (from [rustup.rs](https://rustup.rs)) and, on Linux, these packages, which are already installed if the Python app works: `libmpv2`, `libespeak-ng1` (Piper's phonemes), `espeak-ng` (the eSpeak engine) and `xclip`.

```bash
cd rust
cargo run --release
```

The first build takes about 6 minutes. It downloads ONNX Runtime (for Piper) and compiles it into the program, which is why the program is about 50 MB. Later builds are quick.

Other options:

| Command | What it does |
|---|---|
| `cargo run --release -- --version` | Print the version |
| `cargo run --release -- --check-engines` | Build a short phrase with every engine and report which work |
| `cargo run --release -- --phonemes "some text"` | Show the phonemes Piper is given (debugging) |
| `cargo run --release -- --options=pronunciations` | Open Options on a tab at start-up (`engines`, `pronunciations`, `filters`, `shortcuts`) |
| `cargo test` | Run the tests |

`TR_MPV_LOG=/path/file.log` writes mpv's log, as it does in the Python app.

Run one version at a time. If both are running, both respond to the shortcuts, and the Python app can overwrite settings the Rust one saved, because Python writes its whole settings file each time it saves.

## Layout

```
src/main.rs            start-up, --version, --check-engines
src/ui.rs              the window (egui)
src/controller.rs      reading, playback and settings, shared by the window, shortcuts and tray
src/player.rs          mpv playback (libmpv loaded at run time)
src/options.rs         the Options window
src/pronunciation.rs   pronunciation rules and links/paths/codes filters (port of pronunciation.py)
src/settings.rs        settings.json, shared with the Python app
src/clipboard.rs       selected text / clipboard, with a time limit
src/hotkeys.rs         global shortcuts
src/tray.rs            tray icon
src/engines/           one file per engine; edge_drm.rs is Microsoft's token, checked against the Python package
tests/fixtures/        what the Python app reads for 397 sample texts; regenerate with tests/make_pronunciation_fixtures.py
```
