import { describe, expect, it } from 'vitest'
import fixture from '../../../../../fixtures/scene-evaluator.json'
import type { InputEvent, Scene } from '../types'
import {
  centeredZoomRange,
  clicksAt,
  cursorAt,
  shortcutAt,
  sourcePointToOutput,
  transformAt,
} from './evaluator'

const scene = fixture.scene as Scene
const events = fixture.events as InputEvent[]

describe('scene evaluator parity', () => {
  it('matches the shared Rust transform and cursor samples', () => {
    for (const sample of fixture.samples) {
      const transform = transformAt(scene, sample.timestampMs)
      const cursor = cursorAt(events, sample.timestampMs)
      expect(transform.scale).toBeCloseTo(sample.scale, 8)
      expect(transform.focus).toEqual(sample.focus)
      expect(cursor?.x).toBeCloseTo(sample.cursor.x, 8)
      expect(cursor?.y).toBeCloseTo(sample.cursor.y, 8)
    }
  })

  it('matches click age and shortcut lifetime', () => {
    const click = clicksAt(events, fixture.clickSample.timestampMs)[0]
    expect(click.ageMs).toBe(fixture.clickSample.ageMs)
    expect(click.radius).toBeCloseTo(fixture.clickSample.radius, 8)
    expect(shortcutAt(events, fixture.shortcutSample.timestampMs)?.keys.join(' + '))
      .toBe(fixture.shortcutSample.label)
  })

  it('maps cropped source coordinates into output space', () => {
    const transform = transformAt({ ...scene, zooms: [] }, 0)
    const topLeft = sourcePointToOutput({ x: 0.1, y: 0.2 }, transform)
    expect(topLeft.x).toBeCloseTo(0, 8)
    expect(topLeft.y).toBeCloseTo(0, 8)
    expect(sourcePointToOutput({ x: 0.9, y: 0.8 }, transform).x).toBeCloseTo(1, 8)
    const mapping = fixture.mappingSample
    const mapped = sourcePointToOutput(mapping.source, transformAt(scene, mapping.timestampMs))
    expect(mapped.x).toBeCloseTo(mapping.output.x, 8)
    expect(mapped.y).toBeCloseTo(mapping.output.y, 8)
  })

  it('creates integer zoom timestamps accepted by the Rust u64 contract', () => {
    expect(centeredZoomRange(0, 7_433, 7_433)).toEqual({
      startMs: 3_217,
      endMs: 4_417,
    })
    expect(centeredZoomRange(0, 50, 50)).toBeNull()
  })
})
