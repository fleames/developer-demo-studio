use std::path::PathBuf;

use async_trait::async_trait;
use project_model::{DisplaySource, Rect};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("screen capture is unavailable: {0}")]
    Unavailable(String),
    #[error("screen capture permission was denied")]
    PermissionDenied,
    #[error("invalid capture region")]
    InvalidRegion,
    #[error("capture is already active")]
    AlreadyActive,
    #[error("no capture is active")]
    NotActive,
    #[error("capture backend failed: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, CaptureError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub source_id: String,
    pub region: Rect,
    pub output_path: PathBuf,
    pub frame_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub duration_ms: u64,
    pub frames_written: u64,
    pub dropped_frames: u64,
}

#[async_trait]
pub trait CaptureBackend: Send + Sync {
    async fn displays(&self) -> Result<Vec<DisplaySource>>;
    async fn start(&self, request: CaptureRequest) -> Result<()>;
    async fn pause(&self) -> Result<()>;
    async fn resume(&self) -> Result<()>;
    async fn stop(&self) -> Result<CaptureSummary>;
    async fn discard(&self) -> Result<()>;
}

pub fn validate_request(request: &CaptureRequest, source: &DisplaySource) -> Result<()> {
    let region = request.region;
    if region.width < 2.0
        || region.height < 2.0
        || !source.bounds.contains(project_model::Point {
            x: region.x,
            y: region.y,
        })
        || !source.bounds.contains(project_model::Point {
            x: region.x + region.width,
            y: region.y + region.height,
        })
    {
        return Err(CaptureError::InvalidRegion);
    }
    Ok(())
}
