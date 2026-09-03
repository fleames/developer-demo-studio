# ADR 0001: Windows-first vertical slice

## Status

Accepted for the first milestone.

## Decision

Developer Demo Studio uses a Tauri 2 shell and React UI around a Rust workspace. Native capture is isolated behind `capture-core`; the first backend uses Windows Graphics Capture and Win32 input observation. The portable project model, analysis engine, renderer contract, and export engine contain no Windows-specific types.

The first supported target is Windows 10 1903 or newer. Windows Graphics Capture supplies display frames; a selected region is represented as a project-space crop. Pointer and click events use the same monotonic session clock and remain separate from source pixels.

macOS will map the same contracts to ScreenCaptureKit and a passive Quartz event tap. This avoids claiming untested macOS support while preserving a direct implementation path.

## Rendering and export

Effects are scene data, never burned into source media. Preview is composed from the source plus scene transforms. Exports are rendered to frames and encoded by a restricted FFmpeg process. GIF uses a generated palette followed by `paletteuse`; presets cap dimensions and frame rate.

Production distributions must bundle a replaceable LGPL-compatible FFmpeg build. GPL-enabled developer binaries, including common `--enable-gpl` builds, are not distributable as the product's LGPL sidecar.

## Privacy boundary

Pixels, window metadata, pointer events, and analysis stay local. Raw typed text is neither captured nor logged. Keyboard metadata is restricted to modifier chords and control/navigation keys. Diagnostic logs may contain durations, counts, and error codes, but never frame contents, project event payloads, window titles, or paths containing user content.

## Project layout

```text
demo.ddp/
  manifest.json
  recording/source.mkv
  metadata/events.jsonl
  scene/scene.json
  thumbnails/
  exports/
```

Manifest and scene updates use write-flush-rename. Event metadata is append-only while recording. An unclean `recording` project is recoverable on next open and finalized with the last valid media timestamp.

## API support

| Capability | Windows MVP | macOS mapping |
| --- | --- | --- |
| Display/window frames | Windows.Graphics.Capture | ScreenCaptureKit |
| Region | display capture + crop | content filter + crop |
| Pointer position | GetCursorPos sampling | CGEvent/NSEvent |
| Click metadata | passive low-level mouse hook | passive Quartz event tap |
| Cursor pixels | disabled when supported | `showsCursor = false` |
| Encoding | restricted FFmpeg sidecar | restricted FFmpeg sidecar |

