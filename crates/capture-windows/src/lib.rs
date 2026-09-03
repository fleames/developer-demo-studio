#[cfg(not(windows))]
compile_error!("capture-windows can only be built for Windows targets");

#[cfg(windows)]
mod implementation {
    use std::{
        error::Error,
        fs::{self, File, OpenOptions},
        io::{BufWriter, Write},
        process::{Child, ChildStdin, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use async_trait::async_trait;
    use capture_core::{
        CaptureBackend, CaptureError, CaptureRequest, CaptureSummary, Result, validate_request,
    };
    use parking_lot::Mutex;
    use project_model::{CaptureSourceKind, DisplaySource, InputEvent, MouseButton, Point, Rect};
    use windows::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
        UI::{
            HiDpi::{GetDpiForSystem, GetDpiForWindow},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_LWIN,
                VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_RETURN, VK_SHIFT, VK_TAB,
            },
            WindowsAndMessaging::GetCursorPos,
        },
    };
    use windows_capture::{
        capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
        frame::Frame,
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings,
            Settings,
        },
        window::Window,
    };

    type HandlerError = Box<dyn Error + Send + Sync>;

    struct EventSink {
        writer: Mutex<BufWriter<File>>,
    }

    impl EventSink {
        fn open(path: &std::path::Path) -> std::io::Result<Self> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            Ok(Self {
                writer: Mutex::new(BufWriter::new(file)),
            })
        }

        fn append(&self, event: &InputEvent) -> std::result::Result<(), HandlerError> {
            let mut writer = self.writer.lock();
            serde_json::to_writer(&mut *writer, event)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            Ok(())
        }
    }

    struct EncoderSink {
        stdin: Option<ChildStdin>,
        child: Child,
    }

    struct FrameFlags {
        encoder: Arc<Mutex<EncoderSink>>,
        crop: Option<(u32, u32, u32, u32)>,
        paused: Arc<AtomicBool>,
        frames: Arc<AtomicU64>,
        frame_interval: Duration,
    }

    struct FrameHandler {
        flags: FrameFlags,
        contiguous: Vec<u8>,
        last_frame: Option<Instant>,
    }

    impl GraphicsCaptureApiHandler for FrameHandler {
        type Flags = FrameFlags;
        type Error = HandlerError;

        fn new(context: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
            Ok(Self {
                flags: context.flags,
                contiguous: Vec::new(),
                last_frame: None,
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame<'_>,
            _capture_control: InternalCaptureControl,
        ) -> std::result::Result<(), Self::Error> {
            if self.flags.paused.load(Ordering::Relaxed) {
                return Ok(());
            }
            let now = Instant::now();
            if self
                .last_frame
                .is_some_and(|last| now.duration_since(last) < self.flags.frame_interval)
            {
                return Ok(());
            }
            self.last_frame = Some(now);
            let buffer = if let Some((left, top, right, bottom)) = self.flags.crop {
                frame.buffer_crop(left, top, right, bottom)?
            } else {
                frame.buffer()?
            };
            let pixels = buffer.as_nopadding_buffer(&mut self.contiguous);
            if let Some(stdin) = self.flags.encoder.lock().stdin.as_mut() {
                stdin.write_all(pixels)?;
                self.flags.frames.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
    }

    struct ActiveCapture {
        control: CaptureControl<FrameHandler, HandlerError>,
        encoder: Arc<Mutex<EncoderSink>>,
        sampler_stop: Arc<AtomicBool>,
        sampler: thread::JoinHandle<()>,
        paused: Arc<AtomicBool>,
        frames: Arc<AtomicU64>,
        frame_rate: u32,
        output_path: std::path::PathBuf,
    }

    enum CaptureTarget {
        Monitor(Monitor),
        Window(Window),
    }

    #[derive(Default)]
    pub struct WindowsCaptureBackend {
        active: Mutex<Option<ActiveCapture>>,
    }

    impl WindowsCaptureBackend {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl CaptureBackend for WindowsCaptureBackend {
        async fn sources(&self) -> Result<Vec<DisplaySource>> {
            enumerate_sources()
        }

        async fn start(&self, request: CaptureRequest) -> Result<()> {
            let mut active = self.active.lock();
            if active.is_some() {
                return Err(CaptureError::AlreadyActive);
            }
            let monitors = Monitor::enumerate().map_err(backend_error)?;
            let displays = display_sources(&monitors)?;
            let windows = Window::enumerate().map_err(backend_error)?;
            let window_sources = window_sources(&windows);
            let (target, source, crop) = if let Some(index) = displays
                .iter()
                .position(|source| source.id == request.source_id)
            {
                validate_request(&request, &displays[index])?;
                let source = &displays[index];
                let left = (request.region.x - source.bounds.x).round() as u32;
                let top = (request.region.y - source.bounds.y).round() as u32;
                (
                    CaptureTarget::Monitor(monitors[index]),
                    source,
                    Some((
                        left,
                        top,
                        left + request.region.width.round() as u32,
                        top + request.region.height.round() as u32,
                    )),
                )
            } else if let Some(index) = window_sources
                .iter()
                .position(|source| source.id == request.source_id)
            {
                let source = &window_sources[index];
                let window = windows
                    .iter()
                    .find(|window| window_id(window) == source.id)
                    .copied()
                    .ok_or_else(|| {
                        CaptureError::Unavailable("selected window no longer exists".into())
                    })?;
                (CaptureTarget::Window(window), source, None)
            } else {
                return Err(CaptureError::Unavailable(
                    "selected display or window no longer exists".into(),
                ));
            };
            let pointer_region = if source.kind == CaptureSourceKind::Window {
                source.bounds
            } else {
                request.region
            };

            if let Some(parent) = request.output_path.parent() {
                fs::create_dir_all(parent).map_err(backend_error)?;
            }
            let width = request.region.width.round() as u32;
            let height = request.region.height.round() as u32;
            let encoder = spawn_encoder(&request.output_path, width, height, request.frame_rate)?;
            let encoder = Arc::new(Mutex::new(encoder));
            let paused = Arc::new(AtomicBool::new(false));
            let frames = Arc::new(AtomicU64::new(0));
            let flags = FrameFlags {
                encoder: encoder.clone(),
                crop,
                paused: paused.clone(),
                frames: frames.clone(),
                frame_interval: Duration::from_secs_f64(1.0 / request.frame_rate.max(1) as f64),
            };
            let control = match target {
                CaptureTarget::Monitor(monitor) => {
                    FrameHandler::start_free_threaded(capture_settings(monitor, flags))
                        .map_err(backend_error)?
                }
                CaptureTarget::Window(window) => {
                    FrameHandler::start_free_threaded(capture_settings(window, flags))
                        .map_err(backend_error)?
                }
            };
            let sampler_stop = Arc::new(AtomicBool::new(false));
            let event_path = request
                .output_path
                .parent()
                .and_then(std::path::Path::parent)
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("metadata/events.jsonl");
            let sampler = spawn_pointer_sampler(
                event_path,
                pointer_region,
                sampler_stop.clone(),
                paused.clone(),
                frames.clone(),
                request.frame_rate,
            )?;
            *active = Some(ActiveCapture {
                control,
                encoder,
                sampler_stop,
                sampler,
                paused,
                frames,
                frame_rate: request.frame_rate,
                output_path: request.output_path,
            });
            Ok(())
        }

        async fn pause(&self) -> Result<()> {
            let active = self.active.lock();
            let capture = active.as_ref().ok_or(CaptureError::NotActive)?;
            capture.paused.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn resume(&self) -> Result<()> {
            let active = self.active.lock();
            let capture = active.as_ref().ok_or(CaptureError::NotActive)?;
            capture.paused.store(false, Ordering::Relaxed);
            Ok(())
        }

        async fn stop(&self) -> Result<CaptureSummary> {
            let capture = self.active.lock().take().ok_or(CaptureError::NotActive)?;
            capture.sampler_stop.store(true, Ordering::Relaxed);
            capture
                .sampler
                .join()
                .map_err(|_| CaptureError::Backend("input sampler panicked".into()))?;
            capture.control.stop().map_err(backend_error)?;
            let mut encoder = capture.encoder.lock();
            encoder.stdin.take();
            let status = encoder.child.wait().map_err(backend_error)?;
            if !status.success() {
                return Err(CaptureError::Backend(format!(
                    "FFmpeg exited with {status}; partial media remains recoverable"
                )));
            }
            let frames_written = capture.frames.load(Ordering::Relaxed);
            Ok(CaptureSummary {
                duration_ms: frames_written * 1_000 / capture.frame_rate.max(1) as u64,
                frames_written,
                dropped_frames: 0,
            })
        }

        async fn discard(&self) -> Result<()> {
            let capture = self.active.lock().take().ok_or(CaptureError::NotActive)?;
            capture.sampler_stop.store(true, Ordering::Relaxed);
            let _ = capture.sampler.join();
            let _ = capture.control.stop();
            let mut encoder = capture.encoder.lock();
            encoder.stdin.take();
            let _ = encoder.child.kill();
            let _ = encoder.child.wait();
            let _ = fs::remove_file(capture.output_path);
            Ok(())
        }
    }

    fn enumerate_sources() -> Result<Vec<DisplaySource>> {
        let monitors = Monitor::enumerate().map_err(backend_error)?;
        let mut sources = display_sources(&monitors)?;
        let windows = Window::enumerate().map_err(backend_error)?;
        sources.extend(window_sources(&windows));
        Ok(sources)
    }

    fn display_sources(monitors: &[Monitor]) -> Result<Vec<DisplaySource>> {
        let primary = Monitor::primary().map_err(backend_error)?;
        let scale_factor = unsafe { GetDpiForSystem() } as f64 / 96.0;
        monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    rcMonitor: RECT::default(),
                    rcWork: RECT::default(),
                    dwFlags: 0,
                };
                unsafe { GetMonitorInfoW(HMONITOR(monitor.as_raw_hmonitor()), &raw mut info) }
                    .ok()
                    .map_err(backend_error)?;
                let rect = info.rcMonitor;
                Ok(DisplaySource {
                    id: format!("monitor-{}", index + 1),
                    name: monitor
                        .name()
                        .unwrap_or_else(|_| format!("Display {}", index + 1)),
                    bounds: Rect {
                        x: rect.left as f64,
                        y: rect.top as f64,
                        width: (rect.right - rect.left) as f64,
                        height: (rect.bottom - rect.top) as f64,
                    },
                    scale_factor,
                    primary: monitor == &primary,
                    kind: CaptureSourceKind::Display,
                    process_name: None,
                })
            })
            .collect()
    }

    fn window_sources(windows: &[Window]) -> Vec<DisplaySource> {
        let mut sources = windows
            .iter()
            .filter_map(|window| {
                let title = window.title().ok()?;
                let rect = window.rect().ok()?;
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                if title.trim().is_empty() || width < 2 || height < 2 {
                    return None;
                }
                let scale_factor =
                    unsafe { GetDpiForWindow(HWND(window.as_raw_hwnd())) } as f64 / 96.0;
                Some(DisplaySource {
                    id: window_id(window),
                    name: title,
                    bounds: Rect {
                        x: rect.left as f64,
                        y: rect.top as f64,
                        width: width as f64,
                        height: height as f64,
                    },
                    scale_factor,
                    primary: false,
                    kind: CaptureSourceKind::Window,
                    process_name: window.process_name().ok(),
                })
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| {
            left.process_name
                .cmp(&right.process_name)
                .then_with(|| left.name.cmp(&right.name))
        });
        sources
    }

    fn window_id(window: &Window) -> String {
        format!("window-{:x}", window.as_raw_hwnd() as usize)
    }

    fn capture_settings<T>(target: T, flags: FrameFlags) -> Settings<FrameFlags, T>
    where
        T: TryInto<GraphicsCaptureItemType>,
    {
        Settings::new(
            target,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            flags,
        )
    }

    fn spawn_encoder(
        path: &std::path::Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<EncoderSink> {
        let mut child = Command::new("ffmpeg")
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
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "ffv1",
            ])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                CaptureError::Unavailable(format!("FFmpeg could not be started: {error}"))
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CaptureError::Backend("FFmpeg input pipe was not created".into()))?;
        Ok(EncoderSink {
            stdin: Some(stdin),
            child,
        })
    }

    fn spawn_pointer_sampler(
        event_path: std::path::PathBuf,
        region: Rect,
        stop: Arc<AtomicBool>,
        paused: Arc<AtomicBool>,
        frames: Arc<AtomicU64>,
        frame_rate: u32,
    ) -> Result<thread::JoinHandle<()>> {
        let sink = Arc::new(EventSink::open(&event_path).map_err(backend_error)?);
        Ok(thread::spawn(move || {
            let mut previous_buttons = [false; 3];
            let mut previous_shortcuts = [false; 12];
            let mut last_clicks: [Option<(Instant, Point)>; 3] = [None; 3];
            let mut previous_position = Point { x: -1.0, y: -1.0 };
            let buttons = [
                (VK_LBUTTON, MouseButton::Left),
                (VK_RBUTTON, MouseButton::Right),
                (VK_MBUTTON, MouseButton::Middle),
            ];
            while !stop.load(Ordering::Relaxed) {
                if !paused.load(Ordering::Relaxed) {
                    let timestamp_ms =
                        frames.load(Ordering::Relaxed) * 1_000 / frame_rate.max(1) as u64;
                    let mut point = POINT::default();
                    if unsafe { GetCursorPos(&mut point) }.is_ok() {
                        let screen = Point {
                            x: point.x as f64,
                            y: point.y as f64,
                        };
                        if region.contains(screen) {
                            let normalized = region.normalize_point(screen);
                            if (normalized.x - previous_position.x).abs() > 0.001
                                || (normalized.y - previous_position.y).abs() > 0.001
                            {
                                let _ = sink.append(&InputEvent::Cursor {
                                    timestamp_ms,
                                    position: normalized,
                                });
                                previous_position = normalized;
                            }
                            for (index, (key, button)) in buttons.iter().enumerate() {
                                let down = unsafe { GetAsyncKeyState(key.0 as i32) } < 0;
                                if down && !previous_buttons[index] {
                                    let count =
                                        if last_clicks[index].is_some_and(|(time, point)| {
                                            time.elapsed() <= Duration::from_millis(500)
                                                && ((point.x - normalized.x).powi(2)
                                                    + (point.y - normalized.y).powi(2))
                                                .sqrt()
                                                    <= 0.03
                                        }) {
                                            2
                                        } else {
                                            1
                                        };
                                    let _ = sink.append(&InputEvent::Click {
                                        timestamp_ms,
                                        position: normalized,
                                        button: *button,
                                        count,
                                    });
                                    last_clicks[index] = Some((Instant::now(), normalized));
                                }
                                previous_buttons[index] = down;
                            }
                        }
                    }
                    let modifiers = [
                        (VK_CONTROL, "Ctrl"),
                        (VK_SHIFT, "Shift"),
                        (VK_MENU, "Alt"),
                        (VK_LWIN, "Win"),
                    ];
                    let triggers = [
                        (VIRTUAL_KEY(0x43), "C", false),
                        (VIRTUAL_KEY(0x46), "F", false),
                        (VIRTUAL_KEY(0x4B), "K", false),
                        (VIRTUAL_KEY(0x50), "P", false),
                        (VIRTUAL_KEY(0x52), "R", false),
                        (VIRTUAL_KEY(0x56), "V", false),
                        (VIRTUAL_KEY(0x58), "X", false),
                        (VIRTUAL_KEY(0x5A), "Z", false),
                        (VK_RETURN, "Enter", true),
                        (VK_ESCAPE, "Esc", true),
                        (VK_TAB, "Tab", true),
                        (VIRTUAL_KEY(0x20), "Space", true),
                    ];
                    let held_modifiers: Vec<String> = modifiers
                        .iter()
                        .filter(|(key, _)| key_down(*key))
                        .map(|(_, name)| (*name).to_string())
                        .collect();
                    for (index, (key, name, allow_without_modifier)) in triggers.iter().enumerate()
                    {
                        let down = key_down(*key);
                        if down
                            && !previous_shortcuts[index]
                            && (*allow_without_modifier || !held_modifiers.is_empty())
                        {
                            let mut keys = held_modifiers.clone();
                            keys.push((*name).to_string());
                            let _ = sink.append(&InputEvent::Shortcut { timestamp_ms, keys });
                        }
                        previous_shortcuts[index] = down;
                    }
                }
                thread::sleep(Duration::from_millis(8));
            }
        }))
    }

    fn backend_error(error: impl std::fmt::Display) -> CaptureError {
        CaptureError::Backend(error.to_string())
    }

    fn key_down(key: VIRTUAL_KEY) -> bool {
        (unsafe { GetAsyncKeyState(key.0 as i32) }) < 0
    }
}

#[cfg(windows)]
pub use implementation::WindowsCaptureBackend;
