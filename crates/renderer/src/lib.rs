use project_model::{InputEvent, Point, Rect, Scene, ZoomEvent};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub focus: Point,
    pub scale: f64,
    pub crop: Rect,
}

pub fn transform_at(scene: &Scene, timestamp_ms: u64) -> ViewTransform {
    let Some(zoom) = scene
        .zooms
        .iter()
        .find(|zoom| timestamp_ms >= zoom.start_ms && timestamp_ms <= zoom.end_ms)
    else {
        return ViewTransform {
            focus: Point { x: 0.5, y: 0.5 },
            scale: 1.0,
            crop: scene.crop,
        };
    };
    let progress = zoom_progress(zoom, timestamp_ms);
    ViewTransform {
        focus: zoom.focus,
        scale: 1.0 + (zoom.scale - 1.0) * smoothstep(progress),
        crop: scene.crop,
    }
}

pub fn cursor_at(events: &[InputEvent], timestamp_ms: u64) -> Option<Point> {
    let mut previous = None;
    let mut next = None;
    for event in events {
        if let InputEvent::Cursor {
            timestamp_ms: event_time,
            position,
        } = event
        {
            if *event_time <= timestamp_ms {
                previous = Some((*event_time, *position));
            } else {
                next = Some((*event_time, *position));
                break;
            }
        }
    }
    match (previous, next) {
        (Some((left_time, left)), Some((right_time, right))) if right_time > left_time => {
            let amount = (timestamp_ms - left_time) as f64 / (right_time - left_time) as f64;
            Some(Point {
                x: left.x + (right.x - left.x) * amount,
                y: left.y + (right.y - left.y) * amount,
            })
        }
        (Some((_, point)), _) => Some(point),
        _ => None,
    }
}

pub fn click_age(click_timestamp_ms: u64, timestamp_ms: u64) -> Option<(u64, f64)> {
    let age = timestamp_ms.checked_sub(click_timestamp_ms)?;
    (age <= 420).then_some((age, 10.0 + age as f64 / 30.0))
}

pub fn shortcut_at(events: &[InputEvent], timestamp_ms: u64) -> Option<&[String]> {
    events.iter().rev().find_map(|event| {
        if let InputEvent::Shortcut {
            timestamp_ms: shortcut_time,
            keys,
        } = event
            && *shortcut_time <= timestamp_ms
            && timestamp_ms - *shortcut_time <= 1_200
        {
            return Some(keys.as_slice());
        }
        None
    })
}

pub fn compose_rgba(
    source: &[u8],
    destination: &mut [u8],
    width: u32,
    height: u32,
    scene: &Scene,
    events: &[InputEvent],
    timestamp_ms: u64,
) {
    assert_eq!(source.len(), destination.len());
    let transform = transform_at(scene, timestamp_ms);
    let width_f = width as f64;
    let height_f = height as f64;
    for y in 0..height {
        for x in 0..width {
            let normalized_x = x as f64 / width_f;
            let normalized_y = y as f64 / height_f;
            let crop_center = Point {
                x: transform.crop.x + transform.crop.width / 2.0,
                y: transform.crop.y + transform.crop.height / 2.0,
            };
            let focus = if transform.scale > 1.0001 {
                transform.focus
            } else {
                crop_center
            };
            let base_x = transform.crop.x + normalized_x * transform.crop.width;
            let base_y = transform.crop.y + normalized_y * transform.crop.height;
            let source_x = (focus.x + (base_x - focus.x) / transform.scale).clamp(0.0, 1.0);
            let source_y = (focus.y + (base_y - focus.y) / transform.scale).clamp(0.0, 1.0);
            let sx = (source_x * (width - 1) as f64).round() as u32;
            let sy = (source_y * (height - 1) as f64).round() as u32;
            let from = ((sy * width + sx) * 4) as usize;
            let to = ((y * width + x) * 4) as usize;
            destination[to..to + 4].copy_from_slice(&source[from..from + 4]);
        }
    }

    for mask in &scene.blur_masks {
        if timestamp_ms >= mask.start_ms && timestamp_ms <= mask.end_ms {
            let left = source_to_output_x(mask.region.x, transform) * width_f;
            let top = source_to_output_y(mask.region.y, transform) * height_f;
            let right = source_to_output_x(mask.region.x + mask.region.width, transform) * width_f;
            let bottom =
                source_to_output_y(mask.region.y + mask.region.height, transform) * height_f;
            pixelate(
                destination,
                width,
                height,
                (left as i32, top as i32, right as i32, bottom as i32),
                mask.intensity.max(4) as u32,
            );
        }
    }

    if let Some(cursor) = cursor_at(events, timestamp_ms) {
        let screen = Point {
            x: source_to_output_x(cursor.x, transform) * width_f,
            y: source_to_output_y(cursor.y, transform) * height_f,
        };
        draw_cursor(destination, width, height, screen);
    }
    if scene.click_emphasis {
        for event in events {
            if let InputEvent::Click {
                timestamp_ms: click_time,
                position,
                ..
            } = event
                && let Some((_, radius)) = click_age(*click_time, timestamp_ms)
            {
                let screen = Point {
                    x: source_to_output_x(position.x, transform) * width_f,
                    y: source_to_output_y(position.y, transform) * height_f,
                };
                draw_ring(destination, width, height, screen, radius);
            }
        }
    }
    if let Some(keys) = shortcut_at(events, timestamp_ms) {
        draw_shortcut_card(destination, width, height, keys);
    }
}

fn draw_shortcut_card(buffer: &mut [u8], width: u32, height: u32, keys: &[String]) {
    let card_width = (keys.iter().map(String::len).sum::<usize>() as u32 * 7
        + keys.len() as u32 * 16)
        .clamp(54, width.saturating_sub(8).max(54));
    let card_height = 30_u32.min(height);
    let left = width.saturating_sub(card_width) / 2;
    let top = height.saturating_sub(card_height + 18);
    for y in top..(top + card_height).min(height) {
        for x in left..(left + card_width).min(width) {
            let index = ((y * width + x) * 4) as usize;
            let border =
                y == top || y + 1 == top + card_height || x == left || x + 1 == left + card_width;
            buffer[index..index + 4].copy_from_slice(if border {
                &[80, 88, 98, 255]
            } else {
                &[15, 18, 23, 230]
            });
        }
    }
    let mut x = left + 10;
    for key in keys {
        let key_width = (key.len() as u32 * 6 + 8).max(14);
        for y in (top + 9)..(top + 21).min(height) {
            for pixel_x in x..(x + key_width).min(width) {
                let index = ((y * width + pixel_x) * 4) as usize;
                buffer[index..index + 4].copy_from_slice(&[225, 230, 236, 255]);
            }
        }
        x += key_width + 6;
    }
}

fn source_to_output_x(value: f64, transform: ViewTransform) -> f64 {
    let center = transform.crop.x + transform.crop.width / 2.0;
    let focus = if transform.scale > 1.0001 {
        transform.focus.x
    } else {
        center
    };
    (focus + (value - focus) * transform.scale - transform.crop.x)
        / transform.crop.width.max(f64::EPSILON)
}

fn source_to_output_y(value: f64, transform: ViewTransform) -> f64 {
    let center = transform.crop.y + transform.crop.height / 2.0;
    let focus = if transform.scale > 1.0001 {
        transform.focus.y
    } else {
        center
    };
    (focus + (value - focus) * transform.scale - transform.crop.y)
        / transform.crop.height.max(f64::EPSILON)
}

fn zoom_progress(zoom: &ZoomEvent, timestamp_ms: u64) -> f64 {
    let duration = zoom.end_ms.saturating_sub(zoom.start_ms).max(1);
    let edge = (duration / 4).clamp(80, 320);
    let elapsed = timestamp_ms.saturating_sub(zoom.start_ms);
    if elapsed < edge {
        elapsed as f64 / edge as f64
    } else if elapsed > duration - edge {
        (duration - elapsed) as f64 / edge as f64
    } else {
        1.0
    }
}

fn smoothstep(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn draw_cursor(buffer: &mut [u8], width: u32, height: u32, point: Point) {
    let x = point.x.round() as i32;
    let y = point.y.round() as i32;
    for row in 0..22 {
        for column in 0..=row / 2 {
            put_pixel(
                buffer,
                width,
                height,
                x + column,
                y + row,
                [245, 247, 250, 255],
            );
            if column == 0 || column == row / 2 {
                put_pixel(
                    buffer,
                    width,
                    height,
                    x + column - 1,
                    y + row,
                    [12, 16, 22, 255],
                );
            }
        }
    }
}

fn pixelate(buffer: &mut [u8], width: u32, height: u32, bounds: (i32, i32, i32, i32), block: u32) {
    let left = bounds.0.clamp(0, width as i32) as u32;
    let top = bounds.1.clamp(0, height as i32) as u32;
    let right = bounds.2.clamp(0, width as i32) as u32;
    let bottom = bounds.3.clamp(0, height as i32) as u32;
    for y in (top..bottom).step_by(block as usize) {
        for x in (left..right).step_by(block as usize) {
            let sample = ((y * width + x) * 4) as usize;
            let color = [buffer[sample], buffer[sample + 1], buffer[sample + 2], 255];
            for py in y..(y + block).min(bottom) {
                for px in x..(x + block).min(right) {
                    let index = ((py * width + px) * 4) as usize;
                    buffer[index..index + 4].copy_from_slice(&color);
                }
            }
        }
    }
}

fn draw_ring(buffer: &mut [u8], width: u32, height: u32, point: Point, radius: f64) {
    let min_x = (point.x - radius - 2.0).floor() as i32;
    let max_x = (point.x + radius + 2.0).ceil() as i32;
    let min_y = (point.y - radius - 2.0).floor() as i32;
    let max_y = (point.y + radius + 2.0).ceil() as i32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let distance = ((x as f64 - point.x).powi(2) + (y as f64 - point.y).powi(2)).sqrt();
            if (distance - radius).abs() < 2.0 {
                put_pixel(buffer, width, height, x, y, [111, 219, 170, 255]);
            }
        }
    }
}

fn put_pixel(buffer: &mut [u8], width: u32, height: u32, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let index = ((y as u32 * width + x as u32) * 4) as usize;
    buffer[index..index + 4].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_model::{Easing, Rect};
    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenFixture {
        scene: Scene,
        events: Vec<InputEvent>,
        samples: Vec<GoldenSample>,
        mapping_sample: GoldenMapping,
        click_sample: GoldenClick,
        shortcut_sample: GoldenShortcut,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenSample {
        timestamp_ms: u64,
        scale: f64,
        focus: Point,
        cursor: Point,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenClick {
        timestamp_ms: u64,
        age_ms: u64,
        radius: f64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenMapping {
        timestamp_ms: u64,
        source: Point,
        output: Point,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenShortcut {
        timestamp_ms: u64,
        label: String,
    }

    #[test]
    fn zoom_eases_in_and_out() {
        let scene = Scene {
            schema_version: 1,
            trim_start_ms: 0,
            trim_end_ms: 1_000,
            crop: Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            zooms: vec![ZoomEvent {
                id: Uuid::nil(),
                start_ms: 100,
                end_ms: 900,
                focus: Point { x: 0.2, y: 0.3 },
                scale: 2.0,
                easing: Easing::EaseInOut,
                generated: true,
            }],
            blur_masks: vec![],
            cursor_smoothing: 0.7,
            click_emphasis: true,
        };
        assert_eq!(transform_at(&scene, 99).scale, 1.0);
        assert_eq!(transform_at(&scene, 500).scale, 2.0);
        assert!(transform_at(&scene, 850).scale < 2.0);
    }

    #[test]
    fn evaluator_matches_shared_golden_fixture() {
        let fixture: GoldenFixture =
            serde_json::from_str(include_str!("../../../fixtures/scene-evaluator.json")).unwrap();
        for sample in fixture.samples {
            let transform = transform_at(&fixture.scene, sample.timestamp_ms);
            let cursor = cursor_at(&fixture.events, sample.timestamp_ms).unwrap();
            assert!((transform.scale - sample.scale).abs() < 0.000_001);
            assert_eq!(transform.focus, sample.focus);
            assert!((cursor.x - sample.cursor.x).abs() < 0.000_001);
            assert!((cursor.y - sample.cursor.y).abs() < 0.000_001);
        }
        let (age, radius) = click_age(400, fixture.click_sample.timestamp_ms).unwrap();
        assert_eq!(age, fixture.click_sample.age_ms);
        assert!((radius - fixture.click_sample.radius).abs() < 0.000_001);
        assert_eq!(
            shortcut_at(&fixture.events, fixture.shortcut_sample.timestamp_ms)
                .unwrap()
                .join(" + "),
            fixture.shortcut_sample.label
        );
        let transform = transform_at(&fixture.scene, fixture.mapping_sample.timestamp_ms);
        assert!(
            (source_to_output_x(fixture.mapping_sample.source.x, transform)
                - fixture.mapping_sample.output.x)
                .abs()
                < 0.000_001
        );
        assert!(
            (source_to_output_y(fixture.mapping_sample.source.y, transform)
                - fixture.mapping_sample.output.y)
                .abs()
                < 0.000_001
        );
    }
}
