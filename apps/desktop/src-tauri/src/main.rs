use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use analysis::{AnalysisOptions, make_beautiful};
use capture_core::{CaptureBackend, CaptureRequest};
use capture_windows::WindowsCaptureBackend;
use directories::UserDirs;
use parking_lot::Mutex;
use project_model::{
    CaptureSourceKind, DisplaySource, InputEvent, Project, ProjectIndex, Recording, Rect, Scene,
};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;

struct AppState {
    capture: WindowsCaptureBackend,
    project: Mutex<Option<Project>>,
    index: Mutex<ProjectIndex>,
    recording: AtomicBool,
    export_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectSnapshot {
    root: PathBuf,
    title: String,
    duration_ms: u64,
    preview_path: Option<PathBuf>,
    preview_error: Option<String>,
    revision: u64,
    scene: Scene,
    event_count: usize,
    events: Vec<InputEvent>,
    media: MediaMetadata,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaMetadata {
    width: u32,
    height: u32,
    frame_rate: u32,
    duration_ms: u64,
}

#[tauri::command]
async fn list_capture_sources(state: State<'_, AppState>) -> Result<Vec<DisplaySource>, String> {
    state
        .capture
        .sources()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_recording(
    state: State<'_, AppState>,
    source_id: String,
    region: Rect,
) -> Result<PathBuf, String> {
    let displays = state
        .capture
        .sources()
        .await
        .map_err(|error| error.to_string())?;
    let source = displays
        .into_iter()
        .find(|display| display.id == source_id)
        .ok_or_else(|| "Selected display is no longer available".to_string())?;
    let region = if source.kind == CaptureSourceKind::Window {
        source.bounds
    } else {
        normalize_region(region, source.bounds)?
    };
    let base = projects_root();
    let root = base.join(format!("demo-{}.ddp", Uuid::new_v4()));
    let media_path = PathBuf::from("recording/source.mkv");
    let project = Project::create(
        &root,
        "Untitled developer demo",
        Recording {
            source,
            region,
            media_path: media_path.clone(),
            duration_ms: 0,
            frame_rate: 30,
        },
    )
    .map_err(|error| error.to_string())?;
    if let Err(error) = state
        .capture
        .start(CaptureRequest {
            source_id,
            region,
            output_path: root.join(media_path),
            frame_rate: 30,
        })
        .await
    {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error.to_string());
    }
    let index_result = {
        let index = state.index.lock();
        index.touch(&project)
    };
    if let Err(error) = index_result {
        let _ = state.capture.discard().await;
        let _ = std::fs::remove_dir_all(&root);
        return Err(error.to_string());
    }
    *state.project.lock() = Some(project);
    state.recording.store(true, Ordering::Release);
    Ok(root)
}

#[tauri::command]
async fn pause_recording(state: State<'_, AppState>) -> Result<(), String> {
    state
        .capture
        .pause()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn resume_recording(state: State<'_, AppState>) -> Result<(), String> {
    state
        .capture
        .resume()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn discard_recording(state: State<'_, AppState>) -> Result<(), String> {
    state
        .capture
        .discard()
        .await
        .map_err(|error| error.to_string())?;
    state.recording.store(false, Ordering::Release);
    if let Some(project) = state.project.lock().take() {
        std::fs::remove_dir_all(project.root).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn stop_recording(state: State<'_, AppState>) -> Result<ProjectSnapshot, String> {
    state.recording.store(false, Ordering::Release);
    let summary = state.capture.stop().await;
    let mut guard = state.project.lock();
    let project = guard.as_mut().ok_or("No project is active")?;
    let summary = match summary {
        Ok(summary) => summary,
        Err(error) => {
            let media_duration =
                probe_media_duration_ms(&project.root.join(&project.manifest.recording.media_path));
            project
                .recover_with_media_duration(media_duration)
                .map_err(|recovery| format!("{error}; recovery failed: {recovery}"))?;
            state
                .index
                .lock()
                .touch(project)
                .map_err(|index| index.to_string())?;
            return snapshot(
                project,
                Some(format!("Capture ended unexpectedly: {error}")),
            );
        }
    };
    project
        .finalize(summary.duration_ms)
        .map_err(|error| error.to_string())?;
    state
        .index
        .lock()
        .touch(project)
        .map_err(|error| error.to_string())?;
    let preview_error = create_preview(project).err();
    snapshot(project, preview_error)
}

#[tauri::command]
fn make_it_beautiful(state: State<'_, AppState>) -> Result<ProjectSnapshot, String> {
    let mut guard = state.project.lock();
    let project = guard.as_mut().ok_or("No project is open")?;
    let events = project.read_events().map_err(|error| error.to_string())?;
    let mut scene = project.scene.clone();
    make_beautiful(&mut scene, &events, AnalysisOptions::default());
    let revision = project.manifest.revision;
    project
        .update_scene(scene, revision)
        .map_err(|error| error.to_string())?;
    state
        .index
        .lock()
        .touch(project)
        .map_err(|error| error.to_string())?;
    snapshot(project, None)
}

#[tauri::command]
fn update_scene(
    state: State<'_, AppState>,
    scene: Scene,
    expected_revision: u64,
) -> Result<ProjectSnapshot, String> {
    let mut guard = state.project.lock();
    let project = guard.as_mut().ok_or("No project is open")?;
    project
        .update_scene(scene, expected_revision)
        .map_err(|error| error.to_string())?;
    state
        .index
        .lock()
        .touch(project)
        .map_err(|error| error.to_string())?;
    snapshot(project, None)
}

#[tauri::command]
fn open_recent_project(state: State<'_, AppState>) -> Result<Option<ProjectSnapshot>, String> {
    let mut paths = state
        .index
        .lock()
        .recent_paths(32)
        .map_err(|error| error.to_string())?;
    if paths.is_empty() {
        let root = projects_root();
        if root.is_dir() {
            for entry in std::fs::read_dir(root)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
            {
                if entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "ddp")
                    && let Ok(project) = Project::open(entry.path())
                {
                    let _ = state.index.lock().touch(&project);
                }
            }
            paths = state
                .index
                .lock()
                .recent_paths(32)
                .map_err(|error| error.to_string())?;
        }
    }
    for path in paths {
        let Ok(mut project) = Project::open(path) else {
            continue;
        };
        let media_duration =
            probe_media_duration_ms(&project.root.join(&project.manifest.recording.media_path));
        project
            .recover_with_media_duration(media_duration)
            .map_err(|error| error.to_string())?;
        state
            .index
            .lock()
            .touch(&project)
            .map_err(|error| error.to_string())?;
        let preview_error = if preview_is_valid(&project) {
            None
        } else {
            create_preview(&project).err()
        };
        let result = snapshot(&project, preview_error)?;
        *state.project.lock() = Some(project);
        return Ok(Some(result));
    }
    Ok(None)
}

#[tauri::command]
async fn export_github_gif(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PathBuf, String> {
    let project = state.project.lock().clone().ok_or("No project is open")?;
    let destination = project.root.join("exports/demo-github.gif");
    let path = destination.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active_export = state.export_cancel.lock();
        if active_export.is_some() {
            return Err("An export is already running".into());
        }
        *active_export = Some(cancelled.clone());
    }
    let task = tauri::async_runtime::spawn_blocking(move || {
        export::render_gif(
            &project,
            &path,
            export::GifPreset::GITHUB,
            cancelled,
            |progress| {
                let _ = app.emit("export-progress", progress);
            },
        )
    })
    .await;
    state.export_cancel.lock().take();
    let result = task.map_err(|error| error.to_string())?;
    result.map_err(|error| error.to_string())?;
    Ok(destination)
}

#[tauri::command]
fn cancel_export(state: State<'_, AppState>) -> bool {
    let guard = state.export_cancel.lock();
    if let Some(cancelled) = guard.as_ref() {
        cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        true
    } else {
        false
    }
}

fn snapshot(project: &Project, preview_error: Option<String>) -> Result<ProjectSnapshot, String> {
    let preview_path = project.root.join("recording/preview.mp4");
    let events = project.read_events().map_err(|error| error.to_string())?;
    let recording = &project.manifest.recording;
    Ok(ProjectSnapshot {
        root: project.root.clone(),
        title: project.manifest.title.clone(),
        duration_ms: project.manifest.recording.duration_ms,
        preview_path: preview_is_valid(project).then_some(preview_path),
        preview_error,
        revision: project.manifest.revision,
        scene: project.scene.clone(),
        event_count: events.len(),
        events,
        media: MediaMetadata {
            width: recording.region.width.round() as u32,
            height: recording.region.height.round() as u32,
            frame_rate: recording.frame_rate,
            duration_ms: recording.duration_ms,
        },
    })
}

fn projects_root() -> PathBuf {
    UserDirs::new()
        .and_then(|dirs| dirs.video_dir().map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("Developer Demo Studio")
}

fn probe_media_duration_ms(path: &std::path::Path) -> Option<u64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let seconds = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    Some((seconds * 1_000.0).round().max(0.0) as u64)
}

fn configure_bundled_media_tools() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = executable.parent() else {
        return;
    };
    if !directory.join("ffmpeg.exe").is_file() || !directory.join("ffprobe.exe").is_file() {
        return;
    }
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let paths =
        std::iter::once(directory.to_path_buf()).chain(std::env::split_paths(&current_path));
    if let Ok(path) = std::env::join_paths(paths) {
        // SAFETY: This runs at process startup before Tauri or capture threads are created.
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn preview_is_valid(project: &Project) -> bool {
    std::fs::metadata(project.root.join("recording/preview.mp4"))
        .is_ok_and(|metadata| metadata.len() > 0)
}

fn create_preview(project: &Project) -> Result<(), String> {
    let source = project.root.join(&project.manifest.recording.media_path);
    let destination = project.root.join("recording/preview.mp4");
    let temporary = project.root.join("recording/preview.tmp.mp4");
    let _ = std::fs::remove_file(&temporary);
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args([
            "-an",
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
            "-c:v",
            "h264_mf",
            "-quality",
            "75",
            "-movflags",
            "+faststart",
        ])
        .arg(&temporary)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Could not create preview: {error}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!(
            "Preview encoder exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let bytes = std::fs::metadata(&temporary)
        .map_err(|error| format!("Preview file was not created: {error}"))?
        .len();
    if bytes == 0 {
        let _ = std::fs::remove_file(&temporary);
        return Err("Preview encoder produced an empty file".into());
    }
    let _ = std::fs::remove_file(&destination);
    std::fs::rename(temporary, destination)
        .map_err(|error| format!("Could not publish preview: {error}"))
}

fn normalize_region(region: Rect, bounds: Rect) -> Result<Rect, String> {
    let x = region
        .x
        .round()
        .clamp(bounds.x, bounds.x + bounds.width - 2.0);
    let y = region
        .y
        .round()
        .clamp(bounds.y, bounds.y + bounds.height - 2.0);
    let max_width = bounds.x + bounds.width - x;
    let max_height = bounds.y + bounds.height - y;
    let width = ((region.width.min(max_width).floor() as u64) & !1).max(2) as f64;
    let height = ((region.height.min(max_height).floor() as u64) & !1).max(2) as f64;
    if width > max_width || height > max_height {
        return Err("The selected recording region is outside the display".into());
    }
    Ok(Rect {
        x,
        y,
        width,
        height,
    })
}

fn main() {
    configure_bundled_media_tools();
    let index = ProjectIndex::open(projects_root().join("index.db"))
        .expect("Could not open the local project index");
    tauri::Builder::default()
        .manage(AppState {
            capture: WindowsCaptureBackend::new(),
            project: Mutex::new(None),
            index: Mutex::new(index),
            recording: AtomicBool::new(false),
            export_cancel: Mutex::new(None),
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.recording.swap(false, Ordering::AcqRel) {
                    api.prevent_close();
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<AppState>();
                        let capture = state.capture.stop().await;
                        if let Some(project) = state.project.lock().as_mut() {
                            match capture {
                                Ok(summary) => {
                                    let _ = project.finalize(summary.duration_ms);
                                }
                                Err(_) => {
                                    let media_duration = probe_media_duration_ms(
                                        &project.root.join(&project.manifest.recording.media_path),
                                    );
                                    let _ = project.recover_with_media_duration(media_duration);
                                }
                            }
                            let _ = state.index.lock().touch(project);
                        }
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.close();
                        }
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_capture_sources,
            start_recording,
            pause_recording,
            resume_recording,
            discard_recording,
            stop_recording,
            make_it_beautiful,
            update_scene,
            open_recent_project,
            export_github_gif,
            cancel_export,
        ])
        .run(tauri::generate_context!())
        .expect("Developer Demo Studio failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_regions_are_clamped_and_even_for_h264() {
        let normalized = normalize_region(
            Rect {
                x: 583.06,
                y: 215.25,
                width: 1_220.78,
                height: 1_119.99,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2_560.0,
                height: 1_440.0,
            },
        )
        .unwrap();
        assert_eq!(normalized.x, 583.0);
        assert_eq!(normalized.y, 215.0);
        assert_eq!(normalized.width, 1_220.0);
        assert_eq!(normalized.height, 1_118.0);
    }

    #[test]
    fn recording_regions_cannot_cross_display_edges() {
        let normalized = normalize_region(
            Rect {
                x: 1_900.0,
                y: 1_000.0,
                width: 500.0,
                height: 500.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1_920.0,
                height: 1_080.0,
            },
        )
        .unwrap();
        assert_eq!(normalized.width, 20.0);
        assert_eq!(normalized.height, 80.0);
    }

    #[test]
    fn preview_encoder_repairs_odd_source_dimensions() {
        if Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("preview-test.ddp");
        let region = Rect {
            x: 0.0,
            y: 0.0,
            width: 321.0,
            height: 181.0,
        };
        let mut project = Project::create(
            &root,
            "Preview test",
            Recording {
                source: DisplaySource {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    bounds: region,
                    scale_factor: 1.0,
                    primary: true,
                    kind: CaptureSourceKind::Display,
                    process_name: None,
                },
                region,
                media_path: "recording/source.mkv".into(),
                duration_ms: 0,
                frame_rate: 15,
            },
        )
        .unwrap();
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=321x181:rate=15:duration=1",
                "-c:v",
                "ffv1",
            ])
            .arg(root.join("recording/source.mkv"))
            .status()
            .unwrap();
        assert!(status.success());
        project.finalize(1_000).unwrap();
        create_preview(&project).unwrap();
        assert!(preview_is_valid(&project));
    }
}
