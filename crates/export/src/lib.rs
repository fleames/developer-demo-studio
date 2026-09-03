use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use project_model::Project;
use tempfile::TempDir;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("export I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("FFmpeg is unavailable; install it or bundle the approved LGPL sidecar")]
    FfmpegUnavailable,
    #[error("FFmpeg {stage} failed with status {status}")]
    FfmpegFailed { stage: &'static str, status: i32 },
    #[error("export was cancelled")]
    Cancelled,
    #[error("source dimensions are invalid")]
    InvalidDimensions,
}

pub type Result<T> = std::result::Result<T, ExportError>;

#[derive(Debug, Clone, Copy)]
pub struct GifPreset {
    pub width: u32,
    pub fps: u32,
    pub colors: u16,
    pub loop_forever: bool,
}

impl GifPreset {
    pub const GITHUB: Self = Self {
        width: 960,
        fps: 15,
        colors: 128,
        loop_forever: true,
    };
}

#[derive(Debug, Clone)]
pub struct ExportResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub frames: u64,
}

pub fn estimated_gif_bytes(width: u32, height: u32, duration_ms: u64, fps: u32) -> u64 {
    let frames = duration_ms as f64 / 1_000.0 * fps as f64;
    (width as f64 * height as f64 * frames * 0.105) as u64
}

pub fn render_gif(
    project: &Project,
    destination: impl AsRef<Path>,
    preset: GifPreset,
    cancelled: Arc<AtomicBool>,
    mut progress: impl FnMut(f32),
) -> Result<ExportResult> {
    ffmpeg_available()?;
    let source_path = project.root.join(&project.manifest.recording.media_path);
    let (width, height) = probe_media_dimensions(&source_path)?;
    let raw_events = project
        .read_events()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let events = analysis::smooth_cursor(
        &raw_events,
        project.scene.cursor_smoothing,
        analysis::AnalysisOptions::default().dead_zone,
    );
    let temp = TempDir::new_in(&project.root)?;
    let intermediate = temp.path().join("composited.mkv");
    let palette = temp.path().join("palette.png");
    let frame_bytes = width as usize * height as usize * 4;
    let duration_ms = project
        .scene
        .trim_end_ms
        .saturating_sub(project.scene.trim_start_ms);
    let expected_frames = (duration_ms as f64 / 1_000.0
        * project.manifest.recording.frame_rate as f64)
        .ceil()
        .max(1.0) as u64;

    let mut decoder = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{}ms", project.scene.trim_start_ms))
        .arg("-i")
        .arg(&source_path)
        .args([
            "-t",
            &format!("{duration_ms}ms"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ExportError::FfmpegUnavailable)?;
    let mut encoder = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &format!("{width}x{height}"),
            "-framerate",
            &project.manifest.recording.frame_rate.to_string(),
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "ffv1",
        ])
        .arg(&intermediate)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ExportError::FfmpegUnavailable)?;

    let mut input = decoder
        .stdout
        .take()
        .ok_or(ExportError::FfmpegUnavailable)?;
    let mut output = encoder.stdin.take().ok_or(ExportError::FfmpegUnavailable)?;
    let mut source_frame = vec![0; frame_bytes];
    let mut composed_frame = vec![0; frame_bytes];
    let mut frames = 0_u64;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = decoder.kill();
            let _ = encoder.kill();
            return Err(ExportError::Cancelled);
        }
        match input.read_exact(&mut source_frame) {
            Ok(()) => {
                let timestamp_ms = project.scene.trim_start_ms
                    + frames * 1_000 / project.manifest.recording.frame_rate as u64;
                renderer::compose_rgba(
                    &source_frame,
                    &mut composed_frame,
                    width,
                    height,
                    &project.scene,
                    &events,
                    timestamp_ms,
                );
                output.write_all(&composed_frame)?;
                frames += 1;
                progress((frames as f32 / expected_frames as f32 * 0.7).min(0.7));
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        }
    }
    drop(output);
    ensure_status("decode", decoder.wait()?)?;
    ensure_status("composite", encoder.wait()?)?;

    let output_height =
        ((height as f64 * preset.width as f64 / width as f64) / 2.0).round() as u32 * 2;
    let palette_filter = format!(
        "fps={},scale={}:{output_height}:flags=lanczos,palettegen=max_colors={}:stats_mode=diff",
        preset.fps, preset.width, preset.colors
    );
    ensure_status(
        "palette generation",
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&intermediate)
            .args(["-vf", &palette_filter])
            .arg(&palette)
            .status()?,
    )?;
    progress(0.82);

    let destination = destination.as_ref();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let gif_filter = format!(
        "fps={},scale={}:{output_height}:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=3",
        preset.fps, preset.width
    );
    ensure_status(
        "GIF encoding",
        Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&intermediate)
            .arg("-i")
            .arg(&palette)
            .args(["-lavfi", &gif_filter, "-loop"])
            .arg(if preset.loop_forever { "0" } else { "-1" })
            .arg(destination)
            .status()?,
    )?;
    progress(1.0);
    Ok(ExportResult {
        path: destination.to_path_buf(),
        bytes: fs::metadata(destination)?.len(),
        frames,
    })
}

fn probe_media_dimensions(path: &Path) -> Result<(u32, u32)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()
        .map_err(|_| ExportError::FfmpegUnavailable)?;
    if !output.status.success() {
        return Err(ExportError::FfmpegFailed {
            stage: "probe",
            status: output.status.code().unwrap_or(-1),
        });
    }
    parse_dimensions(String::from_utf8_lossy(&output.stdout).trim())
}

fn parse_dimensions(value: &str) -> Result<(u32, u32)> {
    let (width, height) = value
        .split_once('x')
        .ok_or(ExportError::InvalidDimensions)?;
    let width = width.parse().map_err(|_| ExportError::InvalidDimensions)?;
    let height = height.parse().map_err(|_| ExportError::InvalidDimensions)?;
    if width == 0 || height == 0 {
        return Err(ExportError::InvalidDimensions);
    }
    Ok((width, height))
}

fn ffmpeg_available() -> Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| ExportError::FfmpegUnavailable)?;
    status
        .success()
        .then_some(())
        .ok_or(ExportError::FfmpegUnavailable)
}

fn ensure_status(stage: &'static str, status: std::process::ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(ExportError::FfmpegFailed {
            stage,
            status: status.code().unwrap_or(-1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_scales_with_duration() {
        let short = estimated_gif_bytes(960, 540, 2_000, 15);
        assert_eq!(estimated_gif_bytes(960, 540, 4_000, 15), short * 2);
    }

    #[test]
    fn github_preset_discourages_huge_gifs() {
        const {
            assert!(GifPreset::GITHUB.width <= 960);
            assert!(GifPreset::GITHUB.fps <= 15);
            assert!(GifPreset::GITHUB.colors <= 128);
        }
    }

    #[test]
    fn media_dimensions_use_the_encoded_stream_size() {
        assert_eq!(parse_dimensions("1349x610").unwrap(), (1349, 610));
        assert!(parse_dimensions("1363,617").is_err());
        assert!(parse_dimensions("0x610").is_err());
    }
}
