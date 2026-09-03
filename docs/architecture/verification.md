# Vertical slice verification

Verified on Windows 10 build 19045 with Rust 1.93.1, Node 24.13.1, and FFmpeg 8.1.1.

## Automated gates

- TypeScript production build and Oxlint pass.
- Rust formatting and strict Clippy (`-D warnings`) pass for the workspace and examples.
- Eight unit tests cover project recovery, coordinate transforms, cursor smoothing, click grouping, zoom easing, and GIF preset sizing.
- A deterministic two-second 320×180 fixture renders 30 composited frames to a 144,787-byte looping GIF, then reopens the project and verifies two editable zoom events.
- A real Windows Graphics Capture smoke test requested roughly two seconds from a centered 640×360 region. Frame pacing produced 56 lossless FFV1 frames (1,866 ms of encoded media) and 15,287,455 bytes of recoverable source media.
- The Tauri development app launches and the release build produces `target/release/devdemo-app.exe`.

## Privacy checks

- Cursor pixels are disabled in source capture and reconstructed from normalized metadata.
- Pointer, mouse-button, and shortcut events are written only to the project event log.
- Shortcut capture persists modifier chords and control/navigation keys only. Unmodified text keys are not captured.
- Smoke-test recordings use temporary directories that are deleted when each test exits.
- No frame or input payload is sent over the network or written to diagnostic logs.

## Distribution media tools

Development resolves `ffmpeg` and `ffprobe` from `PATH`. Tagged Windows releases download an immutable BtbN FFmpeg 8.1 LGPL build, verify its pinned SHA-256 digest, and package the separate executables beside the application. Runtime prepends that application directory before capture threads start. The release includes the upstream license, corresponding-source location, build-script location, and archive checksum; GPL and nonfree builds are not distributed.

## Performance boundary

Source recording uses GPU capture but currently copies cropped RGBA frames to the lossless encoder. Final compositing is deterministic CPU rendering. This proves the non-destructive pipeline but is not the final high-resolution performance target; D3D11 texture sharing and GPU compositing remain the next renderer optimization.
