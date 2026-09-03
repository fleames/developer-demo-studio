import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Aperture, CircleStop, Download, EyeOff, MousePointer2, Pause,
  Play, Redo2, RotateCcw, Scissors, ShieldCheck, Sparkles, Undo2, ZoomIn,
} from 'lucide-react'
import { PreviewCanvas } from './editor/preview/PreviewCanvas'
import type {
  BlurMask,
  Display,
  Point,
  ProjectSnapshot as Project,
  Rect,
  Scene,
  Zoom,
} from './editor/types'
import './App.css'
type Phase = 'ready' | 'countdown' | 'recording' | 'paused' | 'processing' | 'editor'

const isDesktop = '__TAURI_INTERNALS__' in window
const seconds = (milliseconds: number) => `${(milliseconds / 1000).toFixed(1)}s`

function App() {
  const [displays, setDisplays] = useState<Display[]>([])
  const [selectedDisplay, setSelectedDisplay] = useState('')
  const [region, setRegion] = useState<Rect>({ x: 0, y: 0, width: 1280, height: 720 })
  const [phase, setPhase] = useState<Phase>('ready')
  const [countdown, setCountdown] = useState(3)
  const [elapsed, setElapsed] = useState(0)
  const [project, setProject] = useState<Project | null>(null)
  const [selectedZoom, setSelectedZoom] = useState<string | null>(null)
  const [playheadMs, setPlayheadMs] = useState(0)
  const [timelineScale, setTimelineScale] = useState(1)
  const [dragStart, setDragStart] = useState<Point | null>(null)
  const [exportProgress, setExportProgress] = useState(0)
  const [saveStatus, setSaveStatus] = useState<'saved' | 'unsaved' | 'saving' | 'error'>('saved')
  const [historyStatus, setHistoryStatus] = useState({ canUndo: false, canRedo: false })
  const revisionRef = useRef(0)
  const saveTimerRef = useRef<number | null>(null)
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve())
  const historyRef = useRef<{ past: Scene[]; future: Scene[] }>({ past: [], future: [] })
  const coalesceRef = useRef<string | null>(null)
  const [notice, setNotice] = useState(
    isDesktop
      ? 'Your recordings stay on this computer.'
      : 'Open the Tauri desktop build to access native screen capture.',
  )
  const display = displays.find((item) => item.id === selectedDisplay)

  function selectRegion(event: ReactPointerEvent<HTMLDivElement>) {
    if (!display) return
    const box = event.currentTarget.getBoundingClientRect()
    const point = {
      x: Math.max(0, Math.min(1, (event.clientX - box.left) / box.width)),
      y: Math.max(0, Math.min(1, (event.clientY - box.top) / box.height)),
    }
    if (!dragStart) {
      setDragStart(point)
      event.currentTarget.setPointerCapture(event.pointerId)
      return
    }
    const left = Math.min(dragStart.x, point.x)
    const top = Math.min(dragStart.y, point.y)
    const width = Math.max(0.02, Math.abs(point.x - dragStart.x))
    const height = Math.max(0.02, Math.abs(point.y - dragStart.y))
    setRegion({
      x: display.bounds.x + left * display.bounds.width,
      y: display.bounds.y + top * display.bounds.height,
      width: width * display.bounds.width,
      height: height * display.bounds.height,
    })
  }

  useEffect(() => {
    if (!isDesktop) {
      return
    }
    invoke<Display[]>('list_displays')
      .then((items) => {
        setDisplays(items)
        const source = items.find((item) => item.primary) ?? items[0]
        if (source) {
          setSelectedDisplay(source.id)
          setRegion(centeredRegion(source.bounds))
        }
      })
      .catch((error) => setNotice(String(error)))
    invoke<Project | null>('open_recent_project')
      .then((recent) => {
        if (!recent) return
        revisionRef.current = recent.revision
        historyRef.current = { past: [], future: [] }
        setHistoryStatus({ canUndo: false, canRedo: false })
        setSaveStatus('saved')
        setProject(recent)
        setPhase('editor')
        setNotice(recent.previewError
          ? `Recording saved, but preview needs attention: ${recent.previewError}`
          : `Reopened ${recent.title} · saved at ${recent.root}`)
      })
      .catch((error) => setNotice(`Could not reopen the recent project: ${String(error)}`))
    const unlisten = listen<number>('export-progress', ({ payload }) => setExportProgress(payload))
    return () => { void unlisten.then((off) => off()) }
  }, [])

  useEffect(() => {
    if (phase !== 'recording') return
    const timer = window.setInterval(() => setElapsed((value) => value + 100), 100)
    return () => window.clearInterval(timer)
  }, [phase])

  useEffect(() => () => {
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current)
  }, [])

  const activeZoom = useMemo(
    () => project?.scene.zooms.find((zoom) => zoom.id === selectedZoom),
    [project, selectedZoom],
  )

  async function beginRecording() {
    if (!selectedDisplay) return
    setPhase('countdown')
    setCountdown(3)
    for (let value = 3; value > 0; value -= 1) {
      setCountdown(value)
      await new Promise((resolve) => window.setTimeout(resolve, 650))
    }
    try {
      await invoke('start_recording', { sourceId: selectedDisplay, region })
      setElapsed(0)
      setPhase('recording')
      setNotice('Recording locally · cursor and clicks captured as editable metadata')
    } catch (error) {
      setNotice(String(error))
      setPhase('ready')
    }
  }

  async function togglePause() {
    const command = phase === 'paused' ? 'resume_recording' : 'pause_recording'
    await invoke(command)
    setPhase(phase === 'paused' ? 'recording' : 'paused')
  }

  async function stopRecording() {
    setPhase('processing')
    try {
      const result = await invoke<Project>('stop_recording')
      acceptSnapshot(result)
      setPhase('editor')
      setNotice(result.previewError
        ? `Recording saved at ${result.root}, but preview failed: ${result.previewError}`
        : `Saved at ${result.root} · found ${result.eventCount} input events`)
    } catch (error) {
      setNotice(String(error))
      setPhase('ready')
    }
  }

  async function discard() {
    await invoke('discard_recording')
    setPhase('ready')
    setElapsed(0)
    setNotice('Recording discarded securely.')
  }

  async function beautify() {
    setNotice('Grouping actions, smoothing cursor, and building camera moves…')
    try {
      if (project && saveStatus !== 'saved') {
        if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current)
        await persistScene(project.scene)
      }
      const result = await invoke<Project>('make_it_beautiful')
      acceptSnapshot(result)
      setSelectedZoom(result.scene.zooms[0]?.id ?? null)
      setNotice(`Your demo is ready · ${result.scene.zooms.length} intelligent zooms`)
    } catch (error) {
      setNotice(`Automatic polish failed: ${String(error)}`)
    }
  }

  function acceptSnapshot(snapshot: Project) {
    revisionRef.current = snapshot.revision
    historyRef.current = { past: [], future: [] }
    setHistoryStatus({ canUndo: false, canRedo: false })
    setSaveStatus('saved')
    setProject(snapshot)
  }

  function saveScene(scene: Scene, coalesceKey?: string) {
    if (!project) return
    const canCoalesce = coalesceKey && coalesceRef.current === coalesceKey
    if (!canCoalesce) {
      historyRef.current.past.push(project.scene)
      if (historyRef.current.past.length > 100) historyRef.current.past.shift()
    }
    historyRef.current.future = []
    setHistoryStatus({ canUndo: historyRef.current.past.length > 0, canRedo: false })
    coalesceRef.current = coalesceKey ?? null
    setProject({ ...project, scene })
    setSaveStatus('unsaved')
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current)
    saveTimerRef.current = window.setTimeout(() => persistScene(scene), 450)
  }

  function persistScene(scene: Scene) {
    saveQueueRef.current = saveQueueRef.current.then(async () => {
      setSaveStatus('saving')
      try {
        const result = await invoke<Project>('update_scene', {
          scene,
          expectedRevision: revisionRef.current,
        })
        revisionRef.current = result.revision
        setProject((current) => current && current.root === result.root
          ? { ...current, revision: result.revision }
          : result)
        setSaveStatus('saved')
      } catch (error) {
        setSaveStatus('error')
        setNotice(`Autosave failed: ${String(error)}`)
      }
    })
    return saveQueueRef.current
  }

  function undo() {
    if (!project) return
    const previous = historyRef.current.past.pop()
    if (!previous) return
    historyRef.current.future.push(project.scene)
    setHistoryStatus({
      canUndo: historyRef.current.past.length > 0,
      canRedo: historyRef.current.future.length > 0,
    })
    setProject({ ...project, scene: previous })
    setSaveStatus('unsaved')
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current)
    saveTimerRef.current = window.setTimeout(() => persistScene(previous), 100)
  }

  function redo() {
    if (!project) return
    const next = historyRef.current.future.pop()
    if (!next) return
    historyRef.current.past.push(project.scene)
    setHistoryStatus({
      canUndo: historyRef.current.past.length > 0,
      canRedo: historyRef.current.future.length > 0,
    })
    setProject({ ...project, scene: next })
    setSaveStatus('unsaved')
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current)
    saveTimerRef.current = window.setTimeout(() => persistScene(next), 100)
  }

  useEffect(() => {
    if (phase !== 'editor') return
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (target?.matches('input, textarea, select')) return
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault()
        const direction = event.key === 'ArrowRight' ? 1 : -1
        const step = event.shiftKey ? 1_000 : 100
        setPlayheadMs((current) => Math.max(
          project?.scene.trimStartMs ?? 0,
          Math.min(project?.scene.trimEndMs ?? 0, current + direction * step),
        ))
        return
      }
      if (!(event.ctrlKey || event.metaKey)) return
      if (event.key.toLowerCase() === 'z') {
        event.preventDefault()
        if (event.shiftKey) redo()
        else undo()
      } else if (event.key.toLowerCase() === 'y') {
        event.preventDefault()
        redo()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  })

  function startTimelineDrag(
    event: ReactPointerEvent<HTMLElement>,
    kind: 'zoom' | 'blur',
    id: string,
  ) {
    if (!project) return
    event.preventDefault()
    event.stopPropagation()
    const target = event.currentTarget
    const track = target.parentElement
    if (!track) return
    const itemBounds = target.getBoundingClientRect()
    const trackBounds = track.getBoundingClientRect()
    const edgeOffset = event.clientX - itemBounds.left
    const mode = edgeOffset < 8 ? 'start' : itemBounds.right - event.clientX < 8 ? 'end' : 'move'
    const originalScene = project.scene
    const source = kind === 'zoom'
      ? originalScene.zooms.find((item) => item.id === id)
      : originalScene.blurMasks.find((item) => item.id === id)
    if (!source) return
    if (kind === 'zoom') setSelectedZoom(id)
    const originX = event.clientX
    let latest = originalScene
    let moved = false

    const onMove = (pointer: PointerEvent) => {
      const rawDelta = (pointer.clientX - originX) / trackBounds.width * project.durationMs
      const delta = pointer.shiftKey ? Math.round(rawDelta) : Math.round(rawDelta / 100) * 100
      moved ||= Math.abs(pointer.clientX - originX) >= 2
      let startMs = source.startMs
      let endMs = source.endMs
      if (mode === 'move') {
        const duration = endMs - startMs
        startMs = Math.max(
          originalScene.trimStartMs,
          Math.min(originalScene.trimEndMs - duration, source.startMs + delta),
        )
        endMs = startMs + duration
      } else if (mode === 'start') {
        startMs = Math.max(
          originalScene.trimStartMs,
          Math.min(endMs - 100, source.startMs + delta),
        )
      } else {
        endMs = Math.min(
          originalScene.trimEndMs,
          Math.max(startMs + 100, source.endMs + delta),
        )
      }
      latest = kind === 'zoom'
        ? {
            ...originalScene,
            zooms: originalScene.zooms.map((item) => item.id === id
              ? { ...item, startMs, endMs, generated: false }
              : item),
          }
        : {
            ...originalScene,
            blurMasks: originalScene.blurMasks.map((item) => item.id === id
              ? { ...item, startMs, endMs }
              : item),
          }
      setProject((current) => current ? { ...current, scene: latest } : current)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      if (moved) saveScene(latest)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp, { once: true })
  }

  function seekFromTimeline(event: ReactPointerEvent<HTMLDivElement>) {
    if (!project) return
    const bounds = event.currentTarget.getBoundingClientRect()
    const fraction = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width))
    setPlayheadMs(project.scene.trimStartMs
      + fraction * (project.scene.trimEndMs - project.scene.trimStartMs))
  }

  function addZoom() {
    if (!project) return
    const startMs = Math.max(project.scene.trimStartMs, project.durationMs / 2 - 500)
    const zoom: Zoom = {
      id: crypto.randomUUID(), startMs, endMs: startMs + 1200,
      focus: { x: 0.5, y: 0.5 }, scale: 1.55, easing: 'easeInOut', generated: false,
    }
    void saveScene({ ...project.scene, zooms: [...project.scene.zooms, zoom] })
    setSelectedZoom(zoom.id)
  }

  function addBlur() {
    if (!project) return
    const mask: BlurMask = {
      id: crypto.randomUUID(), startMs: project.scene.trimStartMs,
      endMs: project.scene.trimEndMs,
      region: { x: 0.65, y: 0.05, width: 0.3, height: 0.12 }, intensity: 12,
    }
    void saveScene({ ...project.scene, blurMasks: [...project.scene.blurMasks, mask] })
  }

  async function exportGif() {
    setExportProgress(0.01)
    setNotice('Rendering GitHub GIF locally…')
    try {
      const path = await invoke<string>('export_github_gif')
      setExportProgress(1)
      setNotice(`Exported ${path}`)
    } catch (error) {
      setExportProgress(0)
      setNotice(String(error))
    }
  }

  async function cancelExport() {
    if (await invoke<boolean>('cancel_export')) {
      setNotice('Cancelling export…')
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand"><Aperture size={18} /><span>Developer Demo Studio</span><b>ALPHA</b></div>
        <nav><button className="nav-active">Project</button><button>Recording</button><button>Export</button></nav>
        <div className="privacy"><ShieldCheck size={15} /> Local only</div>
      </header>

      {phase === 'ready' || phase === 'countdown' ? (
        <section className="start-view">
          <div className="eyebrow">NEW DEVELOPER DEMO</div>
          <h1>Show what you built.</h1>
          <p>Record a focused region. We’ll turn cursor movement and clicks into a polished, editable demo.</p>
          <div className="source-card">
            <label>Capture source
              <select value={selectedDisplay} onChange={(event) => {
                setSelectedDisplay(event.target.value)
                const next = displays.find((item) => item.id === event.target.value)
                if (next) setRegion(centeredRegion(next.bounds))
              }}>
                {displays.map((item) => <option key={item.id} value={item.id}>{item.name} · {item.bounds.width}×{item.bounds.height}</option>)}
              </select>
            </label>
            {display && <div
              className="region-map"
              style={{ aspectRatio: `${display.bounds.width}/${display.bounds.height}` }}
              onPointerDown={selectRegion}
              onPointerMove={(event) => { if (dragStart) selectRegion(event) }}
              onPointerUp={() => setDragStart(null)}
              aria-label="Drag to select the recording region"
            >
              <span className="region-selection" style={{
                left: `${(region.x - display.bounds.x) / display.bounds.width * 100}%`,
                top: `${(region.y - display.bounds.y) / display.bounds.height * 100}%`,
                width: `${region.width / display.bounds.width * 100}%`,
                height: `${region.height / display.bounds.height * 100}%`,
              }}><small>RECORD REGION</small></span>
              <b>{display.name}</b>
            </div>}
            <div className="region-grid">
              {(['x', 'y', 'width', 'height'] as const).map((key) => (
                <label key={key}>{key.toUpperCase()}
                  <input type="number" value={Math.round(region[key])} onChange={(event) =>
                    setRegion({ ...region, [key]: Number(event.target.value) })} />
                </label>
              ))}
            </div>
            <div className="source-meta">
              <MousePointer2 size={15} /> Cursor and clicks stay editable
              <span>{display?.scaleFactor.toFixed(2) ?? '1.00'}× scale</span>
            </div>
          </div>
          <button className="primary record-button" disabled={!isDesktop || !selectedDisplay} onClick={beginRecording}>
            <span className="record-dot" /> Start region recording <kbd>Ctrl R</kbd>
          </button>
          <div className="privacy-note"><EyeOff size={15} /> No account. No upload. No raw typing captured.</div>
          {phase === 'countdown' && <div className="countdown"><span>{countdown}</span><small>Get ready</small></div>}
        </section>
      ) : phase === 'recording' || phase === 'paused' ? (
        <section className="recording-view">
          <div className="recording-orbit"><span><i />REC</span><strong>{seconds(elapsed)}</strong><small>{Math.round(region.width)} × {Math.round(region.height)} · 30 FPS</small></div>
          <div className="record-controls">
            <button onClick={togglePause}>{phase === 'paused' ? <Play /> : <Pause />}{phase === 'paused' ? 'Resume' : 'Pause'}</button>
            <button className="stop" onClick={stopRecording}><CircleStop />Stop & process</button>
            <button onClick={discard}><RotateCcw />Discard</button>
          </div>
        </section>
      ) : phase === 'processing' ? (
        <section className="processing"><Sparkles /><h2>Structuring your demo…</h2><p>Finalizing source media and semantic input events.</p><div className="indeterminate" /></section>
      ) : project && (
        <section className="editor">
          <div className="workspace">
            <div className="preview-toolbar"><span>{project.title}</span><div>
              <button onClick={undo} disabled={!historyStatus.canUndo}><Undo2 /> Undo</button>
              <button onClick={redo} disabled={!historyStatus.canRedo}><Redo2 /> Redo</button>
              <button onClick={addBlur}><EyeOff /> Add blur</button>
              <button onClick={addZoom}><ZoomIn /> Add zoom</button>
            </div></div>
            <div className="stage">
              <div className="presentation-frame">
                {project.previewPath ? <PreviewCanvas
                    source={convertFileSrc(project.previewPath)}
                    scene={project.scene}
                    events={project.events}
                    media={project.media}
                    seekToMs={playheadMs}
                    onTimeChange={setPlayheadMs}
                    onWarning={setNotice}
                  /> : <div className="preview-fallback"><EyeOff /><strong>Preview unavailable</strong>
                    <span>{project.previewError ?? 'The source recording remains safely stored.'}</span>
                    <code>{project.root}</code>
                  </div>}
              </div>
            </div>
            <div className="timeline">
              <div className="timeline-head"><button><Play size={15} /></button><span>00:00</span><div className="ruler" /><span>{seconds(project.durationMs)}</span>
                <label className="timeline-zoom">Zoom <input aria-label="Timeline zoom" type="range" min="1" max="4" step=".25" value={timelineScale} onChange={(event) => setTimelineScale(Number(event.target.value))} /></label>
              </div>
              <div className="track"><label>ZOOM</label><div className="track-line" style={{ minWidth: `${timelineScale * 100}%` }} onPointerDown={seekFromTimeline}>
                <span className="timeline-playhead" style={{ left: `${playheadMs / Math.max(project.durationMs, 1) * 100}%` }} />
                {project.scene.zooms.map((zoom) => <button key={zoom.id} className={selectedZoom === zoom.id ? 'event selected' : 'event'}
                  style={{ left: `${zoom.startMs / Math.max(project.durationMs, 1) * 100}%`, width: `${(zoom.endMs - zoom.startMs) / Math.max(project.durationMs, 1) * 100}%` }}
                  onPointerDown={(event) => startTimelineDrag(event, 'zoom', zoom.id)}
                  onClick={() => setSelectedZoom(zoom.id)}><ZoomIn size={12} />{zoom.generated ? 'Auto' : 'Manual'}</button>)}
              </div></div>
              <div className="track"><label>PRIVACY</label><div className="track-line" style={{ minWidth: `${timelineScale * 100}%` }} onPointerDown={seekFromTimeline}>
                {project.scene.blurMasks.map((mask) => <button key={mask.id} className="event blur"
                  onPointerDown={(event) => startTimelineDrag(event, 'blur', mask.id)}
                  style={{ left: `${mask.startMs / Math.max(project.durationMs, 1) * 100}%`, width: `${(mask.endMs - mask.startMs) / Math.max(project.durationMs, 1) * 100}%` }}>Blur</button>)}
              </div></div>
            </div>
          </div>
          <aside className="inspector">
            <button className="beautiful" onClick={beautify}><Sparkles /> Make it beautiful</button>
            <div className="analysis-stat"><strong>{project.eventCount}</strong><span>input events detected</span></div>
            <section><h3><Scissors /> Trim</h3>
              <label>Start <input type="number" min={0} max={project.scene.trimEndMs} value={project.scene.trimStartMs}
                onChange={(event) => void saveScene({ ...project.scene, trimStartMs: Number(event.target.value) }, 'trim-start')} /> ms</label>
              <label>End <input type="number" min={project.scene.trimStartMs} max={project.durationMs} value={project.scene.trimEndMs}
                onChange={(event) => void saveScene({ ...project.scene, trimEndMs: Number(event.target.value) }, 'trim-end')} /> ms</label>
            </section>
            {activeZoom && <section><h3><ZoomIn /> Zoom</h3>
              <label>Intensity <b>{activeZoom.scale.toFixed(2)}×</b>
                <input type="range" min="1.1" max="2.2" step="0.05" value={activeZoom.scale} onChange={(event) => {
                  const zooms = project.scene.zooms.map((zoom) => zoom.id === activeZoom.id ? { ...zoom, scale: Number(event.target.value) } : zoom)
                  void saveScene({ ...project.scene, zooms }, `zoom-scale-${activeZoom.id}`)
                }} />
              </label>
              <button className="danger-link" onClick={() => {
                void saveScene({ ...project.scene, zooms: project.scene.zooms.filter((zoom) => zoom.id !== activeZoom.id) })
                setSelectedZoom(null)
              }}>Delete zoom</button>
            </section>}
            <section><h3><MousePointer2 /> Cursor</h3><label>Smoothing <b>{Math.round(project.scene.cursorSmoothing * 100)}%</b>
              <input type="range" min="0" max="0.95" step="0.05" value={project.scene.cursorSmoothing}
                onChange={(event) => void saveScene({ ...project.scene, cursorSmoothing: Number(event.target.value) }, 'cursor-smoothing')} /></label>
            </section>
            <section className="export-panel"><h3><Download /> GitHub GIF</h3><p>960px · 15 FPS · 128 colors · loop</p>
              {exportProgress > 0 && exportProgress < 1 && <progress value={exportProgress} max={1} />}
              {exportProgress > 0 && exportProgress < 1
                ? <button className="danger-link export-cancel" onClick={cancelExport}>Cancel export</button>
                : <button className="primary" onClick={exportGif}>Export optimized GIF</button>}
            </section>
          </aside>
        </section>
      )}
      <footer className="statusbar"><span className={notice.includes('error') ? 'error' : ''}>{notice}</span><span>{saveStatus === 'saved' ? 'Saved' : saveStatus === 'saving' ? 'Saving…' : saveStatus === 'error' ? 'Save failed' : 'Unsaved changes'} · DDP v1</span></footer>
    </main>
  )
}

function centeredRegion(bounds: Rect): Rect {
  const width = Math.min(1280, Math.floor(bounds.width * 0.8))
  const height = Math.min(720, Math.floor(bounds.height * 0.8))
  return {
    x: bounds.x + Math.floor((bounds.width - width) / 2),
    y: bounds.y + Math.floor((bounds.height - height) / 2),
    width,
    height,
  }
}

export default App
