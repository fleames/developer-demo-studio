use std::time::Duration;

use capture_core::{CaptureBackend, CaptureRequest};
use capture_windows::WindowsCaptureBackend;
use project_model::Rect;

#[tokio::main]
async fn main() {
    let backend = WindowsCaptureBackend::new();
    let displays = backend.displays().await.expect("enumerate displays");
    let display = displays
        .iter()
        .find(|display| display.primary)
        .or_else(|| displays.first())
        .expect("at least one display");
    let width = display.bounds.width.min(640.0);
    let height = display.bounds.height.min(360.0);
    let region = Rect {
        x: display.bounds.x + (display.bounds.width - width) / 2.0,
        y: display.bounds.y + (display.bounds.height - height) / 2.0,
        width,
        height,
    };
    let temporary = tempfile::tempdir().expect("temporary project");
    let output = temporary.path().join("demo.ddp/recording/source.mkv");
    backend
        .start(CaptureRequest {
            source_id: display.id.clone(),
            region,
            output_path: output.clone(),
            frame_rate: 30,
        })
        .await
        .expect("start capture");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let summary = backend.stop().await.expect("stop capture");
    let bytes = std::fs::metadata(output).expect("source media").len();
    let event_log = temporary.path().join("demo.ddp/metadata/events.jsonl");
    let max_event_time = std::fs::read_to_string(event_log)
        .expect("event metadata")
        .lines()
        .map(|line| serde_json::from_str::<project_model::InputEvent>(line).expect("valid event"))
        .map(|event| event.timestamp_ms())
        .max()
        .unwrap_or(0);
    assert!(summary.frames_written > 0, "capture returned no frames");
    assert!(bytes > 1_024, "source media is unexpectedly small");
    assert!(
        max_event_time <= summary.duration_ms,
        "input metadata extends beyond the encoded media"
    );
    println!(
        "captured {} frames in {} ms ({} bytes)",
        summary.frames_written, summary.duration_ms, bytes
    );
}
