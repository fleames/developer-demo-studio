use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

use analysis::{AnalysisOptions, make_beautiful};
use project_model::{
    CaptureSourceKind, DisplaySource, InputEvent, MouseButton, Point, Project, Recording,
    RecordingState, Rect,
};

fn main() {
    let temporary = tempfile::tempdir().expect("temporary workspace");
    let root = temporary.path().join("fixture.ddp");
    let region = Rect {
        x: 0.0,
        y: 0.0,
        width: 320.0,
        height: 180.0,
    };
    let mut project = Project::create(
        &root,
        "Vertical slice fixture",
        Recording {
            source: DisplaySource {
                id: "fixture".into(),
                name: "Generated fixture".into(),
                bounds: region,
                scale_factor: 1.0,
                primary: true,
                kind: CaptureSourceKind::Display,
                process_name: None,
            },
            region,
            media_path: PathBuf::from("recording/source.mkv"),
            duration_ms: 0,
            frame_rate: 15,
        },
    )
    .expect("create project");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=15:duration=2",
            "-c:v",
            "ffv1",
        ])
        .arg(root.join("recording/source.mkv"))
        .status()
        .expect("generate source fixture");
    assert!(status.success());
    for event in [
        InputEvent::Cursor {
            timestamp_ms: 350,
            position: Point { x: 0.25, y: 0.4 },
        },
        InputEvent::Click {
            timestamp_ms: 500,
            position: Point { x: 0.25, y: 0.4 },
            button: MouseButton::Left,
            count: 1,
        },
        InputEvent::Cursor {
            timestamp_ms: 1_200,
            position: Point { x: 0.78, y: 0.65 },
        },
        InputEvent::Click {
            timestamp_ms: 1_350,
            position: Point { x: 0.78, y: 0.65 },
            button: MouseButton::Left,
            count: 1,
        },
    ] {
        project.append_event(&event).expect("append event");
    }
    project.finalize(2_000).expect("finalize");
    let events = project.read_events().expect("read events");
    make_beautiful(&mut project.scene, &events, AnalysisOptions::default());
    project.save().expect("save generated scene");
    let output = root.join("exports/demo.gif");
    let result = export::render_gif(
        &project,
        &output,
        export::GifPreset {
            width: 320,
            fps: 15,
            ..export::GifPreset::GITHUB
        },
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .expect("export GIF");
    let reopened = Project::open(&root).expect("reopen project");
    assert_eq!(reopened.manifest.state, RecordingState::Ready);
    assert_eq!(reopened.scene.zooms.len(), 2);
    assert!(result.bytes > 1_024);
    println!(
        "rendered {} frames into {} bytes",
        result.frames, result.bytes
    );
}
