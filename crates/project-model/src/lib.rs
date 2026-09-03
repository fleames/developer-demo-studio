use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("project I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("project JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported project schema {found}; newest supported schema is {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("local project index failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("scene revision conflict: expected {expected}, current revision is {current}")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("project directory has no parent")]
    MissingParent,
}

pub type Result<T> = std::result::Result<T, ProjectError>;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x <= self.x + self.width
            && point.y <= self.y + self.height
    }

    pub fn normalize_point(self, point: Point) -> Point {
        Point {
            x: ((point.x - self.x) / self.width).clamp(0.0, 1.0),
            y: ((point.y - self.y) / self.height).clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySource {
    pub id: String,
    pub name: String,
    pub bounds: Rect,
    pub scale_factor: f64,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingState {
    Recording,
    Ready,
    Recovered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub source: DisplaySource,
    pub region: Rect,
    pub media_path: PathBuf,
    pub duration_ms: u64,
    pub frame_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputEvent {
    Cursor {
        #[serde(rename = "timestampMs", alias = "timestamp_ms")]
        timestamp_ms: u64,
        position: Point,
    },
    Click {
        #[serde(rename = "timestampMs", alias = "timestamp_ms")]
        timestamp_ms: u64,
        position: Point,
        button: MouseButton,
        count: u8,
    },
    Shortcut {
        #[serde(rename = "timestampMs", alias = "timestamp_ms")]
        timestamp_ms: u64,
        keys: Vec<String>,
    },
    Paused {
        #[serde(rename = "timestampMs", alias = "timestamp_ms")]
        timestamp_ms: u64,
    },
    Resumed {
        #[serde(rename = "timestampMs", alias = "timestamp_ms")]
        timestamp_ms: u64,
    },
}

impl InputEvent {
    pub fn timestamp_ms(&self) -> u64 {
        match self {
            Self::Cursor { timestamp_ms, .. }
            | Self::Click { timestamp_ms, .. }
            | Self::Shortcut { timestamp_ms, .. }
            | Self::Paused { timestamp_ms }
            | Self::Resumed { timestamp_ms } => *timestamp_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomEvent {
    pub id: Uuid,
    pub start_ms: u64,
    pub end_ms: u64,
    pub focus: Point,
    pub scale: f64,
    pub easing: Easing,
    pub generated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurMask {
    pub id: Uuid,
    pub start_ms: u64,
    pub end_ms: u64,
    pub region: Rect,
    pub intensity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Easing {
    EaseInOut,
    Spring,
    Linear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub schema_version: u32,
    pub trim_start_ms: u64,
    pub trim_end_ms: u64,
    pub crop: Rect,
    pub zooms: Vec<ZoomEvent>,
    #[serde(default)]
    pub blur_masks: Vec<BlurMask>,
    pub cursor_smoothing: f64,
    pub click_emphasis: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub revision: u64,
    pub state: RecordingState,
    pub recording: Recording,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: ProjectManifest,
    pub scene: Scene,
}

impl Project {
    pub fn create(
        root: impl AsRef<Path>,
        title: impl Into<String>,
        recording: Recording,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        for directory in ["recording", "metadata", "scene", "thumbnails", "exports"] {
            fs::create_dir_all(root.join(directory))?;
        }

        let now = Utc::now();
        let manifest = ProjectManifest {
            schema_version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            title: title.into(),
            created_at: now,
            updated_at: now,
            revision: 0,
            state: RecordingState::Recording,
            recording,
        };
        let scene = Scene {
            schema_version: SCHEMA_VERSION,
            trim_start_ms: 0,
            trim_end_ms: 0,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            zooms: Vec::new(),
            blur_masks: Vec::new(),
            cursor_smoothing: 0.72,
            click_emphasis: true,
        };
        let project = Self {
            root,
            manifest,
            scene,
        };
        project.save()?;
        Ok(project)
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let manifest: ProjectManifest = read_json(root.join("manifest.json"))?;
        let mut scene: Scene = read_json(root.join("scene/scene.json"))
            .or_else(|_| read_json(root.join("scene/recovery.json")))?;
        ensure_schema(manifest.schema_version)?;
        ensure_schema(scene.schema_version)?;
        if scene.crop.width > 1.0 || scene.crop.height > 1.0 {
            let source = manifest.recording.region;
            scene.crop = Rect {
                x: ((scene.crop.x - source.x) / source.width).clamp(0.0, 1.0),
                y: ((scene.crop.y - source.y) / source.height).clamp(0.0, 1.0),
                width: (scene.crop.width / source.width).clamp(0.0, 1.0),
                height: (scene.crop.height / source.height).clamp(0.0, 1.0),
            };
            scene.crop.width = scene.crop.width.min(1.0 - scene.crop.x);
            scene.crop.height = scene.crop.height.min(1.0 - scene.crop.y);
        }
        Ok(Self {
            root,
            manifest,
            scene,
        })
    }

    pub fn save(&self) -> Result<()> {
        atomic_json(&self.root.join("manifest.json"), &self.manifest)?;
        atomic_json(&self.root.join("scene/scene.json"), &self.scene)?;
        atomic_json(&self.root.join("scene/recovery.json"), &self.scene)?;
        Ok(())
    }

    pub fn update_scene(&mut self, scene: Scene, expected_revision: u64) -> Result<u64> {
        if expected_revision != self.manifest.revision {
            return Err(ProjectError::RevisionConflict {
                expected: expected_revision,
                current: self.manifest.revision,
            });
        }
        self.scene = scene;
        self.manifest.revision += 1;
        self.manifest.updated_at = Utc::now();
        self.save()?;
        Ok(self.manifest.revision)
    }

    pub fn finalize(&mut self, duration_ms: u64) -> Result<()> {
        self.manifest.recording.duration_ms = duration_ms;
        self.scene.trim_end_ms = duration_ms;
        self.manifest.state = RecordingState::Ready;
        self.manifest.updated_at = Utc::now();
        self.save()
    }

    pub fn recover(&mut self) -> Result<bool> {
        self.recover_with_media_duration(None)
    }

    pub fn recover_with_media_duration(&mut self, media_duration_ms: Option<u64>) -> Result<bool> {
        if self.manifest.state != RecordingState::Recording {
            return Ok(false);
        }
        let events = self.read_events()?;
        let event_duration = events.last().map(InputEvent::timestamp_ms).unwrap_or(0);
        let duration = match (event_duration, media_duration_ms) {
            (0, Some(media)) => media,
            (events, Some(media)) => events.min(media),
            (events, None) => events,
        };
        self.manifest.recording.duration_ms = duration;
        self.scene.trim_end_ms = duration;
        self.manifest.state = if duration == 0 {
            RecordingState::Failed
        } else {
            RecordingState::Recovered
        };
        self.manifest.updated_at = Utc::now();
        self.save()?;
        Ok(true)
    }

    pub fn append_event(&self, event: &InputEvent) -> Result<()> {
        let path = self.root.join("metadata/events.jsonl");
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, event)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    pub fn read_events(&self) -> Result<Vec<InputEvent>> {
        let path = self.root.join("metadata/events.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str(&line) {
                events.push(event);
            }
        }
        Ok(events)
    }
}

pub struct ProjectIndex {
    connection: Connection,
}

impl ProjectIndex {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS recent_projects (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               path TEXT NOT NULL UNIQUE,
               updated_at TEXT NOT NULL
             );",
        )?;
        Ok(Self { connection })
    }

    pub fn touch(&self, project: &Project) -> Result<()> {
        self.connection.execute(
            "DELETE FROM recent_projects WHERE id = ?1 OR path = ?2",
            params![
                project.manifest.id.to_string(),
                project.root.to_string_lossy()
            ],
        )?;
        self.connection.execute(
            "INSERT INTO recent_projects (id, title, path, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                project.manifest.id.to_string(),
                project.manifest.title,
                project.root.to_string_lossy(),
                project.manifest.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn recent_paths(&self, limit: usize) -> Result<Vec<PathBuf>> {
        let mut statement = self
            .connection
            .prepare("SELECT path FROM recent_projects ORDER BY updated_at DESC LIMIT ?1")?;
        let rows = statement.query_map([limit as i64], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(PathBuf::from(row?))).collect()
    }
}

fn ensure_schema(found: u32) -> Result<()> {
    if found > SCHEMA_VERSION {
        return Err(ProjectError::UnsupportedSchema {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    Ok(serde_json::from_reader(BufReader::new(File::open(path)?))?)
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or(ProjectError::MissingParent)?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> Recording {
        Recording {
            source: DisplaySource {
                id: "display-1".into(),
                name: "Primary display".into(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                },
                scale_factor: 1.0,
                primary: true,
            },
            region: Rect {
                x: 100.0,
                y: 100.0,
                width: 800.0,
                height: 500.0,
            },
            media_path: "recording/source.mkv".into(),
            duration_ms: 0,
            frame_rate: 30,
        }
    }

    #[test]
    fn project_round_trip_and_recovery_are_non_destructive() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("demo.ddp");
        let project = Project::create(&root, "Demo", recording()).unwrap();
        project
            .append_event(&InputEvent::Cursor {
                timestamp_ms: 425,
                position: Point { x: 0.5, y: 0.4 },
            })
            .unwrap();

        let mut reopened = Project::open(&root).unwrap();
        assert!(reopened.recover().unwrap());
        assert_eq!(reopened.manifest.recording.duration_ms, 425);
        assert_eq!(reopened.manifest.state, RecordingState::Recovered);
        assert_eq!(reopened.read_events().unwrap().len(), 1);
    }

    #[test]
    fn coordinate_normalization_is_clamped() {
        let rect = recording().region;
        assert_eq!(
            rect.normalize_point(Point { x: 500.0, y: 350.0 }),
            Point { x: 0.5, y: 0.5 }
        );
        assert_eq!(
            rect.normalize_point(Point { x: -10.0, y: 900.0 }),
            Point { x: 0.0, y: 1.0 }
        );
    }

    #[test]
    fn revisioned_scene_save_rejects_stale_writes_and_recovers_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("revision.ddp");
        let mut project = Project::create(&root, "Revision", recording()).unwrap();
        let mut scene = project.scene.clone();
        scene.cursor_smoothing = 0.4;
        assert_eq!(project.update_scene(scene.clone(), 0).unwrap(), 1);
        assert!(matches!(
            project.update_scene(scene, 0),
            Err(ProjectError::RevisionConflict {
                expected: 0,
                current: 1
            })
        ));

        fs::write(root.join("scene/scene.json"), b"{not valid json").unwrap();
        let recovered = Project::open(&root).unwrap();
        assert_eq!(recovered.manifest.revision, 1);
        assert_eq!(recovered.scene.cursor_smoothing, 0.4);
    }

    #[test]
    fn project_index_tracks_manifest_recency() {
        let temp = tempfile::tempdir().unwrap();
        let first = Project::create(temp.path().join("first.ddp"), "First", recording()).unwrap();
        let mut second =
            Project::create(temp.path().join("second.ddp"), "Second", recording()).unwrap();
        second.manifest.updated_at += chrono::Duration::seconds(1);
        second.save().unwrap();
        let index = ProjectIndex::open(temp.path().join("index.db")).unwrap();
        index.touch(&first).unwrap();
        index.touch(&second).unwrap();
        assert_eq!(index.recent_paths(1).unwrap(), vec![second.root]);
    }
}
