import type { ViewTransform } from '../scene/evaluator'
import type { BlurMask } from '../types'

const vertexShader = `#version 300 es
in vec2 a_position;
out vec2 v_uv;
void main() {
  v_uv = a_position * 0.5 + 0.5;
  gl_Position = vec4(a_position, 0.0, 1.0);
}`

const fragmentShader = `#version 300 es
precision highp float;
uniform sampler2D u_video;
uniform vec4 u_crop;
uniform vec2 u_focus;
uniform float u_scale;
uniform vec2 u_video_size;
uniform int u_blur_count;
uniform vec4 u_blurs[8];
uniform float u_blur_blocks[8];
in vec2 v_uv;
out vec4 out_color;
void main() {
  vec2 crop_center = u_crop.xy + u_crop.zw * 0.5;
  vec2 focus = u_scale > 1.0001 ? u_focus : crop_center;
  vec2 base = u_crop.xy + v_uv * u_crop.zw;
  vec2 source_uv = focus + (base - focus) / u_scale;
  for (int index = 0; index < 8; index++) {
    if (index >= u_blur_count) break;
    vec4 mask = u_blurs[index];
    if (source_uv.x >= mask.x && source_uv.y >= mask.y
        && source_uv.x <= mask.x + mask.z && source_uv.y <= mask.y + mask.w) {
      vec2 blocks = max(vec2(1.0), u_video_size / u_blur_blocks[index]);
      source_uv = (floor(source_uv * blocks) + 0.5) / blocks;
    }
  }
  out_color = texture(u_video, clamp(source_uv, vec2(0.0), vec2(1.0)));
}`

export class WebGlVideoRenderer {
  private readonly gl: WebGL2RenderingContext
  private readonly program: WebGLProgram
  private readonly texture: WebGLTexture
  private readonly cropLocation: WebGLUniformLocation
  private readonly focusLocation: WebGLUniformLocation
  private readonly scaleLocation: WebGLUniformLocation
  private readonly videoSizeLocation: WebGLUniformLocation
  private readonly blurCountLocation: WebGLUniformLocation
  private readonly blursLocation: WebGLUniformLocation
  private readonly blurBlocksLocation: WebGLUniformLocation

  static create(canvas: HTMLCanvasElement): WebGlVideoRenderer | null {
    const gl = canvas.getContext('webgl2', {
      alpha: false,
      antialias: false,
      desynchronized: true,
      powerPreference: 'high-performance',
      preserveDrawingBuffer: false,
    })
    return gl ? new WebGlVideoRenderer(gl) : null
  }

  private constructor(gl: WebGL2RenderingContext) {
    this.gl = gl
    this.program = createProgram(gl)
    gl.useProgram(this.program)

    const buffer = gl.createBuffer()
    if (!buffer) throw new Error('Could not allocate preview vertex buffer')
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
      gl.STATIC_DRAW,
    )
    const position = gl.getAttribLocation(this.program, 'a_position')
    gl.enableVertexAttribArray(position)
    gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0)

    const texture = gl.createTexture()
    if (!texture) throw new Error('Could not allocate preview video texture')
    this.texture = texture
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true)

    this.cropLocation = uniform(gl, this.program, 'u_crop')
    this.focusLocation = uniform(gl, this.program, 'u_focus')
    this.scaleLocation = uniform(gl, this.program, 'u_scale')
    this.videoSizeLocation = uniform(gl, this.program, 'u_video_size')
    this.blurCountLocation = uniform(gl, this.program, 'u_blur_count')
    this.blursLocation = uniform(gl, this.program, 'u_blurs[0]')
    this.blurBlocksLocation = uniform(gl, this.program, 'u_blur_blocks[0]')
  }

  render(video: HTMLVideoElement, transform: ViewTransform, masks: BlurMask[]): void {
    const gl = this.gl
    gl.viewport(0, 0, gl.canvas.width, gl.canvas.height)
    gl.useProgram(this.program)
    gl.bindTexture(gl.TEXTURE_2D, this.texture)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      video,
    )
    gl.uniform4f(
      this.cropLocation,
      transform.crop.x,
      transform.crop.y,
      transform.crop.width,
      transform.crop.height,
    )
    gl.uniform2f(this.focusLocation, transform.focus.x, transform.focus.y)
    gl.uniform1f(this.scaleLocation, transform.scale)
    gl.uniform2f(this.videoSizeLocation, video.videoWidth, video.videoHeight)
    const active = masks.slice(0, 8)
    const regions = new Float32Array(8 * 4)
    const blocks = new Float32Array(8)
    active.forEach((mask, index) => {
      regions.set([mask.region.x, mask.region.y, mask.region.width, mask.region.height], index * 4)
      blocks[index] = mask.intensity
    })
    gl.uniform1i(this.blurCountLocation, active.length)
    gl.uniform4fv(this.blursLocation, regions)
    gl.uniform1fv(this.blurBlocksLocation, blocks)
    gl.drawArrays(gl.TRIANGLES, 0, 6)
  }
}

function createProgram(gl: WebGL2RenderingContext): WebGLProgram {
  const program = gl.createProgram()
  if (!program) throw new Error('Could not create preview shader program')
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, vertexShader))
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, fragmentShader))
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) ?? 'Preview shader link failed')
  }
  return program
}

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type)
  if (!shader) throw new Error('Could not allocate preview shader')
  gl.shaderSource(shader, source)
  gl.compileShader(shader)
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) ?? 'Preview shader compilation failed')
  }
  return shader
}

function uniform(
  gl: WebGL2RenderingContext,
  program: WebGLProgram,
  name: string,
): WebGLUniformLocation {
  const location = gl.getUniformLocation(program, name)
  if (!location) throw new Error(`Preview shader uniform ${name} is missing`)
  return location
}
