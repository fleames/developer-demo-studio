use project_model::{Easing, InputEvent, Point, Scene, ZoomEvent};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    pub smoothing: f64,
    pub dead_zone: f64,
    pub group_window_ms: u64,
    pub lead_in_ms: u64,
    pub hold_ms: u64,
    pub max_scale: f64,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            smoothing: 0.72,
            dead_zone: 0.025,
            group_window_ms: 850,
            lead_in_ms: 220,
            hold_ms: 1_200,
            max_scale: 1.85,
        }
    }
}

pub fn make_beautiful(
    scene: &mut Scene,
    events: &[InputEvent],
    options: AnalysisOptions,
) -> Vec<InputEvent> {
    let smoothed = smooth_cursor(events, options.smoothing, options.dead_zone);
    let mut zooms = scene
        .zooms
        .iter()
        .filter(|zoom| !zoom.generated)
        .cloned()
        .collect::<Vec<_>>();
    let analysis_events = smoothed
        .iter()
        .filter(|event| event.timestamp_ms() <= scene.trim_end_ms)
        .cloned()
        .collect::<Vec<_>>();
    zooms.extend(
        generate_zooms(&analysis_events, options)
            .into_iter()
            .filter_map(|mut zoom| {
                zoom.end_ms = zoom.end_ms.min(scene.trim_end_ms);
                (zoom.start_ms < zoom.end_ms).then_some(zoom)
            }),
    );
    zooms.sort_by_key(|zoom| zoom.start_ms);
    scene.zooms = zooms;
    scene.cursor_smoothing = options.smoothing;
    smoothed
}

pub fn smooth_cursor(events: &[InputEvent], smoothing: f64, dead_zone: f64) -> Vec<InputEvent> {
    let alpha = (1.0 - smoothing).clamp(0.02, 1.0);
    let mut result = Vec::with_capacity(events.len());
    let mut current: Option<Point> = None;
    for event in events {
        if let InputEvent::Cursor {
            timestamp_ms,
            position,
        } = event
        {
            let next = match current {
                None => *position,
                Some(previous) => {
                    let distance = ((position.x - previous.x).powi(2)
                        + (position.y - previous.y).powi(2))
                    .sqrt();
                    if distance < dead_zone {
                        previous
                    } else {
                        Point {
                            x: previous.x + (position.x - previous.x) * alpha,
                            y: previous.y + (position.y - previous.y) * alpha,
                        }
                    }
                }
            };
            current = Some(next);
            result.push(InputEvent::Cursor {
                timestamp_ms: *timestamp_ms,
                position: next,
            });
        } else {
            result.push(event.clone());
        }
    }
    result
}

pub fn generate_zooms(events: &[InputEvent], options: AnalysisOptions) -> Vec<ZoomEvent> {
    let mut groups: Vec<Vec<(u64, Point)>> = Vec::new();
    for event in events {
        let InputEvent::Click {
            timestamp_ms,
            position,
            ..
        } = event
        else {
            continue;
        };
        let belongs_to_last = groups.last().and_then(|group| group.last()).is_some_and(
            |(last_time, last_position)| {
                timestamp_ms.saturating_sub(*last_time) <= options.group_window_ms
                    && distance(*last_position, *position) <= 0.22
            },
        );
        if belongs_to_last {
            groups.last_mut().unwrap().push((*timestamp_ms, *position));
        } else {
            groups.push(vec![(*timestamp_ms, *position)]);
        }
    }

    let mut zooms: Vec<ZoomEvent> = groups
        .into_iter()
        .map(|group| {
            let first = group.first().unwrap().0;
            let last = group.last().unwrap().0;
            let count = group.len() as f64;
            let focus = Point {
                x: group.iter().map(|(_, point)| point.x).sum::<f64>() / count,
                y: group.iter().map(|(_, point)| point.y).sum::<f64>() / count,
            };
            ZoomEvent {
                id: Uuid::new_v4(),
                start_ms: first.saturating_sub(options.lead_in_ms),
                end_ms: last + options.hold_ms,
                focus,
                scale: (1.45 + (count - 1.0) * 0.08).min(options.max_scale),
                easing: Easing::Spring,
                generated: true,
            }
        })
        .collect();

    for index in 1..zooms.len() {
        if zooms[index].start_ms < zooms[index - 1].end_ms {
            let boundary = (zooms[index].start_ms + zooms[index - 1].end_ms) / 2;
            zooms[index - 1].end_ms = boundary;
            zooms[index].start_ms = boundary;
        }
    }
    zooms
}

fn distance(left: Point, right: Point) -> f64 {
    ((left.x - right.x).powi(2) + (left.y - right.y).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_model::{MouseButton, Rect};

    fn click(timestamp_ms: u64, x: f64, y: f64) -> InputEvent {
        InputEvent::Click {
            timestamp_ms,
            position: Point { x, y },
            button: MouseButton::Left,
            count: 1,
        }
    }

    #[test]
    fn nearby_clicks_form_one_bounded_zoom() {
        let zooms = generate_zooms(
            &[click(1_000, 0.4, 0.5), click(1_400, 0.43, 0.52)],
            AnalysisOptions::default(),
        );
        assert_eq!(zooms.len(), 1);
        assert!(zooms[0].scale <= AnalysisOptions::default().max_scale);
        assert_eq!(zooms[0].start_ms, 780);
    }

    #[test]
    fn distant_clicks_do_not_make_overlapping_zooms() {
        let zooms = generate_zooms(
            &[click(1_000, 0.1, 0.1), click(1_500, 0.9, 0.9)],
            AnalysisOptions::default(),
        );
        assert_eq!(zooms.len(), 2);
        assert!(zooms[0].end_ms <= zooms[1].start_ms);
    }

    #[test]
    fn make_beautiful_replaces_only_generated_scene_data() {
        let mut scene = Scene {
            schema_version: 1,
            trim_start_ms: 0,
            trim_end_ms: 2_000,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            zooms: vec![ZoomEvent {
                id: Uuid::nil(),
                start_ms: 100,
                end_ms: 300,
                focus: Point { x: 0.2, y: 0.2 },
                scale: 1.3,
                easing: Easing::Linear,
                generated: false,
            }],
            blur_masks: vec![],
            cursor_smoothing: 0.0,
            click_emphasis: true,
        };
        make_beautiful(
            &mut scene,
            &[click(500, 0.5, 0.5), click(2_500, 0.8, 0.8)],
            AnalysisOptions::default(),
        );
        assert_eq!(scene.zooms.len(), 2);
        assert!(scene.zooms.iter().any(|zoom| !zoom.generated));
        assert!(scene.zooms.iter().all(|zoom| zoom.end_ms <= 2_000));
        assert_eq!(scene.trim_end_ms, 2_000);
    }
}
