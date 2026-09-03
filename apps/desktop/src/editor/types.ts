export type Point = { x: number; y: number }
export type Rect = Point & { width: number; height: number }

export type Display = {
  id: string
  name: string
  bounds: Rect
  scaleFactor: number
  primary: boolean
}

export type Zoom = {
  id: string
  startMs: number
  endMs: number
  focus: Point
  scale: number
  easing: 'easeInOut' | 'spring' | 'linear'
  generated: boolean
}

export type BlurMask = {
  id: string
  startMs: number
  endMs: number
  region: Rect
  intensity: number
}

export type Scene = {
  schemaVersion: number
  trimStartMs: number
  trimEndMs: number
  crop: Rect
  zooms: Zoom[]
  blurMasks: BlurMask[]
  cursorSmoothing: number
  clickEmphasis: boolean
}

export type CursorEvent = {
  type: 'cursor'
  timestampMs: number
  position: Point
}

export type ClickEvent = {
  type: 'click'
  timestampMs: number
  position: Point
  button: 'left' | 'right' | 'middle'
  count: number
}

export type ShortcutEvent = {
  type: 'shortcut'
  timestampMs: number
  keys: string[]
}

export type PauseEvent = {
  type: 'paused' | 'resumed'
  timestampMs: number
}

export type InputEvent = CursorEvent | ClickEvent | ShortcutEvent | PauseEvent

export type MediaMetadata = {
  width: number
  height: number
  frameRate: number
  durationMs: number
}

export type ProjectSnapshot = {
  root: string
  title: string
  durationMs: number
  previewPath: string | null
  previewError: string | null
  revision: number
  scene: Scene
  eventCount: number
  events: InputEvent[]
  media: MediaMetadata
}
