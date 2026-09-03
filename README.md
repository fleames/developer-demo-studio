<div align="center">
  <img src="apps/desktop/src/assets/hero.png" width="180" alt="Developer Demo Studio" />
  <h1>Developer Demo Studio</h1>
  <p><strong>Turn a development action into a polished, editable technical demo.</strong></p>

  [![CI](https://github.com/fleames/developer-demo-studio/actions/workflows/ci.yml/badge.svg)](https://github.com/fleames/developer-demo-studio/actions/workflows/ci.yml)
  [![Release](https://img.shields.io/github/v/release/fleames/developer-demo-studio?include_prereleases&color=7c4dff)](https://github.com/fleames/developer-demo-studio/releases)
  [![License: MIT](https://img.shields.io/badge/License-MIT-6fdbad.svg)](LICENSE)
  ![Platform](https://img.shields.io/badge/platform-Windows-2980b9)
</div>

Developer Demo Studio records a focused region, captures cursor intent as editable metadata, and turns the result into a presentation-ready demo. Source pixels remain untouched: zooms, blur masks, cursor motion, clicks, shortcuts, crop, and trim are composed non-destructively.

> [!IMPORTANT]
> This project is an early Windows-first alpha. It is suitable for local testing, not production distribution.

## Highlights

- **Focused capture** — record a selected display region with Windows Graphics Capture.
- **Editable interactions** — cursor positions, clicks, and safe shortcut chords remain structured metadata.
- **Real-time preview** — WebGL2 composition driven by decoded video-frame timestamps, with a Canvas2D fallback.
- **Production-style timeline** — scrub, trim, drag, resize, snap, zoom, undo, and redo.
- **Crash-safe projects** — atomic saves, revision checks, recovery snapshots, media-aware recovery, and SQLite recents.
- **Local export** — deterministic, optimized GitHub GIF rendering through FFmpeg.
- **Private by design** — no account, cloud upload, analytics, or raw typing capture.

## How it works

```text
Windows Graphics Capture ──> lossless source.mkv
Win32 input sampling ──────> metadata/events.jsonl
                                  │
Scene editor ──────────────> scene/scene.json
                                  │
                  ┌───────────────┴───────────────┐
                  ▼                               ▼
        WebGL2 live preview              Rust export compositor
```

The TypeScript preview evaluator and Rust export renderer share checked-in golden fixtures so crop, camera movement, cursor interpolation, click timing, blur geometry, and shortcuts cannot silently drift.

## Run locally

### Requirements

- Windows 10 or newer
- Node.js 24+
- Rust 1.93+
- FFmpeg and FFprobe available on `PATH`
- WebView2 Runtime

```powershell
git clone https://github.com/fleames/developer-demo-studio.git
cd developer-demo-studio
npm ci
npm run tauri --workspace desktop -- dev
```

Run the complete verification suite:

```powershell
npm test
npm run lint
cargo clippy --workspace --all-targets -- -D warnings
```

Build the Windows executable:

```powershell
npm run tauri --workspace desktop -- build
```

The executable is written to `target/release/devdemo-app.exe`.

## Project structure

```text
apps/desktop/          React editor and Tauri application
crates/analysis/       Cursor smoothing and semantic zoom generation
crates/capture-*/      Capture contracts and Windows implementation
crates/project-model/  Portable project format, indexing, and recovery
crates/renderer/       Deterministic frame compositor
crates/export/         Optimized GIF pipeline
fixtures/              Shared preview/export parity fixtures
docs/architecture/     Architecture decisions and verification notes
```

## Releases

GitHub releases are automated from tags matching `v*`. Alpha binaries do not bundle FFmpeg; install an FFmpeg build appropriate for your environment and license obligations.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Architecture details live in [the vertical-slice ADR](docs/architecture/0001-vertical-slice.md).

## License

Developer Demo Studio is available under the [MIT License](LICENSE). FFmpeg is a separate project distributed under its own license.
