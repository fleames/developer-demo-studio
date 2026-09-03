# Contributing

Thanks for helping improve Developer Demo Studio.

## Development workflow

1. Create a focused branch from `main`.
2. Keep capture, project-model, renderer, and UI changes separated when practical.
3. Add or update tests for behavior changes.
4. Run the required checks:

```powershell
npm ci
npm test
npm run lint
npm run build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

5. Open a pull request describing the user-visible result and test coverage.

## Design constraints

- Keep recordings and metadata local unless a future feature explicitly obtains consent.
- Never capture unmodified text keystrokes.
- Preserve source media; editor operations must remain non-destructive.
- Keep TypeScript preview calculations aligned with the Rust renderer and shared fixtures.
- Do not commit generated recordings, exports, credentials, or third-party FFmpeg binaries.

## Reporting bugs

Include Windows version, GPU, WebView2 version, FFmpeg version, reproduction steps, and any visible error message. Do not attach recordings containing private information.
