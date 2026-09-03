import type { ClickEvent, InputEvent, Point, Rect, Scene, ShortcutEvent, Zoom } from '../types'

export type ViewTransform = {
  focus: Point
  scale: number
  crop: Rect
}

export type EvaluatedClick = ClickEvent & { ageMs: number; radius: number }

export function transformAt(scene: Scene, timestampMs: number): ViewTransform {
  const zoom = scene.zooms.find(
    (candidate) => timestampMs >= candidate.startMs && timestampMs <= candidate.endMs,
  )
  if (!zoom) {
    return { focus: { x: 0.5, y: 0.5 }, scale: 1, crop: scene.crop }
  }
  const progress = zoomProgress(zoom, timestampMs)
  return {
    focus: zoom.focus,
    scale: 1 + (zoom.scale - 1) * smoothstep(progress),
    crop: scene.crop,
  }
}

export function cursorAt(events: InputEvent[], timestampMs: number): Point | null {
  let previous: { timestampMs: number; position: Point } | null = null
  let next: { timestampMs: number; position: Point } | null = null
  for (const event of events) {
    if (event.type !== 'cursor') continue
    if (event.timestampMs <= timestampMs) previous = event
    else {
      next = event
      break
    }
  }
  if (!previous) return null
  if (!next || next.timestampMs <= previous.timestampMs) return previous.position
  const amount = (timestampMs - previous.timestampMs) / (next.timestampMs - previous.timestampMs)
  return {
    x: previous.position.x + (next.position.x - previous.position.x) * amount,
    y: previous.position.y + (next.position.y - previous.position.y) * amount,
  }
}

export function clicksAt(events: InputEvent[], timestampMs: number): EvaluatedClick[] {
  return events
    .filter((event): event is ClickEvent => event.type === 'click')
    .filter((event) => timestampMs >= event.timestampMs && timestampMs - event.timestampMs <= 420)
    .map((event) => {
      const ageMs = timestampMs - event.timestampMs
      return { ...event, ageMs, radius: 10 + ageMs / 30 }
    })
}

export function shortcutAt(events: InputEvent[], timestampMs: number): ShortcutEvent | null {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index]
    if (event.type !== 'shortcut' || event.timestampMs > timestampMs) continue
    return timestampMs - event.timestampMs <= 1_200 ? event : null
  }
  return null
}

export function sourcePointToOutput(point: Point, transform: ViewTransform): Point {
  const baseCenter = {
    x: transform.crop.x + transform.crop.width / 2,
    y: transform.crop.y + transform.crop.height / 2,
  }
  const cameraFocus = transform.scale === 1 ? baseCenter : transform.focus
  return {
    x: (cameraFocus.x + (point.x - cameraFocus.x) * transform.scale - transform.crop.x)
      / Math.max(Number.EPSILON, transform.crop.width),
    y: (cameraFocus.y + (point.y - cameraFocus.y) * transform.scale - transform.crop.y)
      / Math.max(Number.EPSILON, transform.crop.height),
  }
}

export function outputToSource(point: Point, transform: ViewTransform): Point {
  const baseCenter = {
    x: transform.crop.x + transform.crop.width / 2,
    y: transform.crop.y + transform.crop.height / 2,
  }
  const cameraFocus = transform.scale === 1 ? baseCenter : transform.focus
  return {
    x: cameraFocus.x
      + (transform.crop.x + point.x * transform.crop.width - cameraFocus.x) / transform.scale,
    y: cameraFocus.y
      + (transform.crop.y + point.y * transform.crop.height - cameraFocus.y) / transform.scale,
  }
}

export function isWithinTrim(scene: Scene, timestampMs: number): boolean {
  return timestampMs >= scene.trimStartMs && timestampMs <= scene.trimEndMs
}

export function centeredZoomRange(
  trimStartMs: number,
  trimEndMs: number,
  durationMs: number,
): { startMs: number; endMs: number } | null {
  const trimStart = Math.round(trimStartMs)
  const trimEnd = Math.round(trimEndMs)
  if (trimEnd - trimStart < 100) return null
  const preferredStart = Math.round(Math.max(trimStart, durationMs / 2 - 500))
  const endMs = Math.min(trimEnd, preferredStart + 1_200)
  const startMs = Math.max(trimStart, Math.min(preferredStart, endMs - 100))
  return { startMs, endMs }
}

function zoomProgress(zoom: Zoom, timestampMs: number): number {
  const duration = Math.max(1, zoom.endMs - zoom.startMs)
  const edge = Math.max(80, Math.min(320, Math.floor(duration / 4)))
  const elapsed = Math.max(0, timestampMs - zoom.startMs)
  if (elapsed < edge) return elapsed / edge
  if (elapsed > duration - edge) return (duration - elapsed) / edge
  return 1
}

function smoothstep(value: number): number {
  const clamped = Math.max(0, Math.min(1, value))
  return clamped * clamped * (3 - 2 * clamped)
}
