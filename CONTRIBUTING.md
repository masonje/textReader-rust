# Contributing to Clipboard Reader (Rust)

This covers working on the code. For installing and using the app, see the [README](README.md).

## Development Setup

Install Rust from [rustup.rs](https://rustup.rs) (no sudo needed) and the system packages listed under [Requirements](README.md#requirements). Then:

```bash
cargo run --release
```

The first build takes about 6 minutes: it downloads ONNX Runtime (for Piper) and compiles it into the program, which is why the program is about 50 MB. Later builds take under a minute. The `target/` build folder grows to several GB.

Use `--release` for anything to do with speech: Piper is much slower in a debug build.

## Project Layout

```
src/main.rs            start-up and the command-line options (--version, --check-engines, ...)
src/ui.rs              the main window: Compact and Full views, word highlighting, Building Audio window (egui)
src/options.rs         the Options window and its five tabs
src/controller.rs      reading, playback, Surprise me and settings; shared by the window, shortcuts and tray
src/player.rs          mpv playback (libmpv loaded at run time)
src/word_timing.rs     when each word is spoken, for the highlighting (port of word_timing.py)
src/pronunciation.rs   pronunciation rules and links/paths/codes filters (port of pronunciation.py)
src/settings.rs        ~/.textReader/settings.json, shared with the Python version
src/clipboard.rs       selected text / clipboard via xclip, with a time limit
src/hotkeys.rs         global shortcuts (watches keys with X RECORD, like the Python version's pynput)
src/tray.rs            tray icon (StatusNotifierItem) and its menu; Show Window brings back a window closed to the tray
src/engines/           one file per engine, plus mod.rs (the Engine trait and the list of engines)
src/engines/edge_drm.rs  Microsoft's Sec-MS-GEC token for Edge TTS, checked against the Python edge-tts package
assets/pireader.png    the app icon (built into the program)
scripts/install.sh     install for this user, with an app-menu launcher
tests/fixtures/        the Python version's results, which the tests compare against
```

## Tests

```bash
cargo test --release
```

The tests cover the pronunciation rules and filters, word timing, the Edge token, gTTS and Edge text splitting, shortcut parsing, and the Options actions (with a temporary settings folder).

The pronunciation and word-timing tests compare against what the Python version produces, stored in `tests/fixtures/`. If the Python version's rules or timing change, regenerate them from a checkout of [the Python version](https://github.com/masonje/textReader) next to this one, with its `.venv` set up:

```bash
cd ../textReader
.venv/bin/python ../textReader-rust/tests/make_pronunciation_fixtures.py
.venv/bin/python ../textReader-rust/tests/make_timing_fixtures.py
```

## Speech Engines

Each engine is a file in `src/engines/` implementing the `Engine` trait in [mod.rs](src/engines/mod.rs). The rest of the app only uses the trait, so it contains no per-engine logic.

### Adding a new engine
1. Create `src/engines/myengine.rs` and add `pub mod myengine;` to `mod.rs`.
2. Implement `Engine`:
	- `name()`: what shows in the Engine drop-down. Use the same name as the Python version's engine, so settings.json is shared.
	- `ext()`: `"wav"` or `"mp3"`. Write whatever the engine produces natively: WAV for engines that run locally, MP3 for online services that send MP3. Converting only adds time.
	- `missing()`: `None` if it can run, or what's missing (shown in Options and by `--check-engines`).
	- `voices()`: `(voice id, label)` pairs. The window shows the labels and passes the chosen id to `synthesize`.
	- `synthesize(text, voice, out, build)`: write audio to `out`. Call `build.check()?` between steps so Cancel works, and use `run_cancellable()` for external programs (it kills them on cancel). Report slow steps with `(build.progress)("…")`; the Building Audio window shows the text, and a trailing `(NN%)` drives its progress bar.
	- Optional: `online()` (the company whose servers make the speech), `quality()`, `about()`, and `download_state()` / `download()` / `delete()` for engines with downloadable voices.
3. Add it to `all()` in `mod.rs`.
4. Check it with `cargo run --release -- --check-engines`.

## Building

`scripts/install.sh` builds the release program, copies it to `~/.local/bin/clipboard-reader`, writes the icon at each size (with the program's own `--write-icons` option, so no image tools are needed) and adds a launcher to the app menu. libmpv, libespeak-ng and xclip aren't built in, so the target system needs them.

After building, check it with `clipboard-reader --version` and `clipboard-reader --check-engines`.

## Debugging

As well as the user-facing [Troubleshooting](README.md#troubleshooting) options:

| Setting | What it does |
|---|---|
| `--phonemes "some text"` | Print the phonemes Piper is given for the text. |
| `--options=<tab>` | Open Options on a tab at start-up (`engines`, `pronunciations`, `filters`, `shortcuts`, `misc`). |
| `--write-icons <folder>` | Write the icon at each size into `<folder>/<n>x<n>/apps/`. |
| `TR_MPV_LOG=/path/file.log` | Write mpv's full playback log to that file. |
| `HOME=/some/test/folder` | Run with separate settings, voices and logs, so testing doesn't touch your own `~/.textReader`. Give a test copy different shortcuts (for example `Ctrl+Alt+Shift+F9`), or it answers the same keys as your real one. |

## Releases

1. Set the version in [Cargo.toml](Cargo.toml) and on the **Version** line of the README.
2. Add an entry at the top of the README's Changelog.
3. Run `cargo test --release` and `scripts/install.sh`, then check `clipboard-reader --version` and `clipboard-reader --check-engines`.
4. Open a pull request into `main`.
