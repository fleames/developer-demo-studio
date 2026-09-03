import { useEffect, useRef, useState } from 'react'
import { Pause, Play, SkipBack } from 'lucide-react'
import {
  clicksAt,
  cursorAt,
  shortcutAt,
  sourcePointToOutput,
  transformAt,
  type ViewTransform,
} from '../scene/evaluator'
import type { BlurMask, InputEvent, MediaMetadata, Scene } from '../types'
import { WebGlVideoRenderer } from './webgl'
import './preview.css'

type Props = {
  source: string
  scene: Scene
  events: InputEvent[]
  media: MediaMetadata
  seekToMs?: number
  togglePlaybackRequest?: number
  onTimeChange?: (timestampMs: number) => void
  onWarning?: (message: string) => void
}

export function PreviewCanvas({
  source,
  scene,
  events,
  media,
  seekToMs,
  togglePlaybackRequest = 0,
  onTimeChange,
  onWarning,
}: Props) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const sourceCanvasRef = useRef<HTMLCanvasElement>(null)
  const overlayRef = useRef<HTMLCanvasElement>(null)
  const rendererBadgeRef = useRef<HTMLSpanElement>(null)
  const webglRef = useRef<WebGlVideoRenderer | null>(null)
  const renderRef = useRef<() => void>(() => undefined)
  const sceneRef = useRef(scene)
  const eventsRef = useRef(events)
  const onTimeChangeRef = useRef(onTimeChange)
  const warningRef = useRef(onWarning)
  const lastPlaybackRequestRef = useRef(togglePlaybackRequest)
  const [playing, setPlaying] = useState(false)
  const [timestampMs, setTimestampMs] = useState(scene.trimStartMs)

  useEffect(() => {
    const canvas = sourceCanvasRef.current
    if (!canvas) return
    try {
      webglRef.current = WebGlVideoRenderer.create(canvas)
      if (!webglRef.current) {
        showFallback(rendererBadgeRef.current)
        warningRef.current?.('WebGL2 is unavailable. Preview is using the slower Canvas2D fallback.')
      }
    } catch (error) {
      webglRef.current = null
      showFallback(rendererBadgeRef.current)
      warningRef.current?.(`GPU preview initialization failed: ${String(error)}`)
    }
  }, [])

  useEffect(() => {
    const video = videoRef.current
    const canvas = sourceCanvasRef.current
    const overlay = overlayRef.current
    if (!video || !canvas || !overlay) return
    let videoFrameId: number | null = null
    let animationFrameId: number | null = null

    const resize = () => {
      const width = Math.max(2, video.videoWidth || media.width)
      const height = Math.max(2, video.videoHeight || media.height)
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width
        canvas.height = height
        overlay.width = width
        overlay.height = height
      }
    }

    const render = () => {
      if (video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return
      resize()
      const currentMs = Math.round(video.currentTime * 1_000)
      const activeScene = sceneRef.current
      const transform = transformAt(activeScene, currentMs)
      const masks = activeScene.blurMasks.filter(
        (mask) => currentMs >= mask.startMs && currentMs <= mask.endMs,
      )
      if (webglRef.current) {
        webglRef.current.render(video, transform, masks)
      } else {
        renderCanvas2d(canvas, video, transform, masks)
      }
      renderOverlay(overlay, transform, eventsRef.current, activeScene, currentMs)
      setTimestampMs(currentMs)
      onTimeChangeRef.current?.(currentMs)
      if (currentMs >= activeScene.trimEndMs && !video.paused) {
        video.pause()
        video.currentTime = activeScene.trimStartMs / 1_000
      }
    }
    renderRef.current = render

    const onFrame = () => {
      render()
      videoFrameId = video.requestVideoFrameCallback(onFrame)
    }
    const onAnimationFrame = () => {
      render()
      animationFrameId = requestAnimationFrame(onAnimationFrame)
    }
    const onLoaded = () => {
      video.currentTime = sceneRef.current.trimStartMs / 1_000
      render()
    }
    const onPlay = () => setPlaying(true)
    const onPause = () => setPlaying(false)
    video.addEventListener('loadeddata', onLoaded)
    video.addEventListener('seeked', render)
    video.addEventListener('play', onPlay)
    video.addEventListener('pause', onPause)
    if ('requestVideoFrameCallback' in video) {
      videoFrameId = video.requestVideoFrameCallback(onFrame)
    } else {
      showFallback(rendererBadgeRef.current)
      warningRef.current?.('Frame-synchronized video callbacks are unavailable; preview timing is approximate.')
      animationFrameId = requestAnimationFrame(onAnimationFrame)
    }
    if (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) onLoaded()

    return () => {
      if (videoFrameId !== null) video.cancelVideoFrameCallback(videoFrameId)
      if (animationFrameId !== null) cancelAnimationFrame(animationFrameId)
      video.removeEventListener('loadeddata', onLoaded)
      video.removeEventListener('seeked', render)
      video.removeEventListener('play', onPlay)
      video.removeEventListener('pause', onPause)
    }
  }, [source, media.height, media.width])

  useEffect(() => {
    sceneRef.current = scene
    eventsRef.current = events
    renderRef.current()
  }, [scene, events])

  useEffect(() => {
    const video = videoRef.current
    if (seekToMs === undefined || !video || Math.abs(video.currentTime * 1_000 - seekToMs) < 25) return
    video.currentTime = seekToMs / 1_000
  }, [seekToMs])

  useEffect(() => {
    if (togglePlaybackRequest === lastPlaybackRequestRef.current) return
    lastPlaybackRequestRef.current = togglePlaybackRequest
    const video = videoRef.current
    if (!video) return
    if (video.paused) {
      if (video.currentTime * 1_000 >= scene.trimEndMs) {
        video.currentTime = scene.trimStartMs / 1_000
      }
      void video.play()
    } else {
      video.pause()
    }
  }, [togglePlaybackRequest, scene.trimEndMs, scene.trimStartMs])

  useEffect(() => {
    onTimeChangeRef.current = onTimeChange
    warningRef.current = onWarning
  }, [onTimeChange, onWarning])

  const seek = (nextMs: number) => {
    const video = videoRef.current
    if (!video) return
    video.currentTime = nextMs / 1_000
    setTimestampMs(nextMs)
  }

  const togglePlayback = async () => {
    const video = videoRef.current
    if (!video) return
    if (video.paused) {
      if (video.currentTime * 1_000 >= scene.trimEndMs) {
        video.currentTime = scene.trimStartMs / 1_000
      }
      await video.play()
    } else {
      video.pause()
    }
  }

  return (
    <div className="preview-canvas-shell" style={{
      aspectRatio: `${media.width * scene.crop.width}/${media.height * scene.crop.height}`,
    }}>
      <video ref={videoRef} src={source} crossOrigin="anonymous" preload="auto" className="preview-decoder" />
      <canvas ref={sourceCanvasRef} className="preview-source-canvas" aria-label="Rendered demo preview" />
      <canvas ref={overlayRef} className="preview-overlay-canvas" aria-hidden="true" />
      <span ref={rendererBadgeRef} className="preview-renderer-badge">GPU</span>
      <div className="preview-playback">
        <button onClick={() => seek(scene.trimStartMs)} aria-label="Return to trim start"><SkipBack /></button>
        <button onClick={togglePlayback} aria-label={playing ? 'Pause preview' : 'Play preview'}>
          {playing ? <Pause /> : <Play />}
        </button>
        <span>{formatTime(timestampMs)}</span>
        <input
          aria-label="Preview playhead"
          type="range"
          min={scene.trimStartMs}
          max={Math.max(scene.trimStartMs + 1, scene.trimEndMs)}
          value={Math.min(timestampMs, scene.trimEndMs)}
          onChange={(event) => seek(Number(event.target.value))}
        />
        <span>{formatTime(scene.trimEndMs)}</span>
      </div>
    </div>
  )
}

function showFallback(badge: HTMLSpanElement | null) {
  if (!badge) return
  badge.textContent = 'CPU'
  badge.classList.add('canvas2d')
}

function renderCanvas2d(
  canvas: HTMLCanvasElement,
  video: HTMLVideoElement,
  transform: ViewTransform,
  masks: BlurMask[],
) {
  const context = canvas.getContext('2d', { alpha: false, desynchronized: true })
  if (!context) return
  const center = transform.scale > 1.0001
    ? transform.focus
    : {
        x: transform.crop.x + transform.crop.width / 2,
        y: transform.crop.y + transform.crop.height / 2,
      }
  const sourceX = center.x + (transform.crop.x - center.x) / transform.scale
  const sourceY = center.y + (transform.crop.y - center.y) / transform.scale
  const sourceWidth = transform.crop.width / transform.scale
  const sourceHeight = transform.crop.height / transform.scale
  context.drawImage(
    video,
    sourceX * video.videoWidth,
    sourceY * video.videoHeight,
    sourceWidth * video.videoWidth,
    sourceHeight * video.videoHeight,
    0,
    0,
    canvas.width,
    canvas.height,
  )
  context.imageSmoothingEnabled = false
  for (const mask of masks) {
    const topLeft = sourcePointToOutput(mask.region, transform)
    const bottomRight = sourcePointToOutput({
      x: mask.region.x + mask.region.width,
      y: mask.region.y + mask.region.height,
    }, transform)
    const x = topLeft.x * canvas.width
    const y = topLeft.y * canvas.height
    const width = (bottomRight.x - topLeft.x) * canvas.width
    const height = (bottomRight.y - topLeft.y) * canvas.height
    if (width <= 0 || height <= 0) continue
    const sample = document.createElement('canvas')
    sample.width = Math.max(1, Math.ceil(width / mask.intensity))
    sample.height = Math.max(1, Math.ceil(height / mask.intensity))
    sample.getContext('2d')?.drawImage(canvas, x, y, width, height, 0, 0, sample.width, sample.height)
    context.drawImage(sample, 0, 0, sample.width, sample.height, x, y, width, height)
  }
  context.imageSmoothingEnabled = true
}

function renderOverlay(
  canvas: HTMLCanvasElement,
  transform: ViewTransform,
  events: InputEvent[],
  scene: Scene,
  timestampMs: number,
) {
  const context = canvas.getContext('2d')
  if (!context) return
  context.clearRect(0, 0, canvas.width, canvas.height)
  if (scene.clickEmphasis) {
    for (const click of clicksAt(events, timestampMs)) {
      const point = sourcePointToOutput(click.position, transform)
      context.strokeStyle = '#6fdbad'
      context.lineWidth = Math.max(2, canvas.width / 640)
      context.beginPath()
      context.arc(point.x * canvas.width, point.y * canvas.height, click.radius, 0, Math.PI * 2)
      context.stroke()
    }
  }
  const cursor = cursorAt(events, timestampMs)
  if (cursor) drawCursor(context, sourcePointToOutput(cursor, transform), canvas)
  const shortcut = shortcutAt(events, timestampMs)
  if (shortcut) drawShortcut(context, shortcut.keys.join(' + '), canvas)
}

function drawCursor(context: CanvasRenderingContext2D, point: { x: number; y: number }, canvas: HTMLCanvasElement) {
  const x = point.x * canvas.width
  const y = point.y * canvas.height
  context.save()
  context.translate(x, y)
  context.scale(Math.max(1, canvas.width / 960), Math.max(1, canvas.width / 960))
  context.beginPath()
  context.moveTo(0, 0)
  context.lineTo(4, 22)
  context.lineTo(9, 15)
  context.lineTo(16, 15)
  context.closePath()
  context.fillStyle = '#f7f8fa'
  context.strokeStyle = '#0b0d11'
  context.lineWidth = 2
  context.fill()
  context.stroke()
  context.restore()
}

function drawShortcut(context: CanvasRenderingContext2D, label: string, canvas: HTMLCanvasElement) {
  context.save()
  context.font = `600 ${Math.max(13, canvas.width / 70)}px ui-monospace, monospace`
  const metrics = context.measureText(label)
  const width = metrics.width + 30
  const height = 38
  const x = (canvas.width - width) / 2
  const y = canvas.height - height - 28
  context.fillStyle = '#0b0e12dd'
  context.strokeStyle = '#3a424d'
  context.lineWidth = 1
  context.beginPath()
  context.roundRect(x, y, width, height, 8)
  context.fill()
  context.stroke()
  context.fillStyle = '#f0f3f6'
  context.textBaseline = 'middle'
  context.fillText(label, x + 15, y + height / 2)
  context.restore()
}

function formatTime(milliseconds: number): string {
  const totalSeconds = Math.max(0, milliseconds) / 1_000
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = (totalSeconds % 60).toFixed(1).padStart(4, '0')
  return `${minutes}:${seconds}`
}
