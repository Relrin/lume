/**
 * Drawing the cursor's effects with Canvas 2D, where WebGL2 cannot be had.
 *
 * The same shapes as the shader draws, with the browser's own paths: a fade along a
 * trail is a gradient, and its glow is the canvas's shadow. The picture is close to the
 * shader's rather than equal to it. Only the rectangles last drawn in are cleared.
 */

import { channels, ShapeKind, type FxBackend, type FxShape } from "./shape.ts";

export class Canvas2dFx implements FxBackend {
  readonly kind = "canvas2d" as const;
  readonly #canvas: HTMLCanvasElement;
  readonly #context: CanvasRenderingContext2D;
  /** The rectangles drawn in last time, as left, top, right, bottom. */
  readonly #drawn: number[] = [];

  constructor(canvas: HTMLCanvasElement) {
    this.#canvas = canvas;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("no Canvas 2D context");
    this.#context = context;
  }

  resize(width: number, height: number): void {
    if (this.#canvas.width !== width) this.#canvas.width = width;
    if (this.#canvas.height !== height) this.#canvas.height = height;
    this.#drawn.length = 0;
  }

  clear(): void {
    const drawn = this.#drawn;
    for (let index = 0; index < drawn.length; index += 4) {
      const left = drawn[index] ?? 0;
      const top = drawn[index + 1] ?? 0;
      this.#context.clearRect(
        left,
        top,
        (drawn[index + 2] ?? 0) - left,
        (drawn[index + 3] ?? 0) - top,
      );
    }
    drawn.length = 0;
  }

  draw(shapes: readonly FxShape[], count: number): void {
    this.clear();
    const context = this.#context;
    for (let index = 0; index < count; index++) {
      const shape = shapes[index];
      if (!shape) continue;
      const left = Math.floor(shape.left);
      const top = Math.floor(shape.top);
      const right = Math.ceil(shape.right);
      const bottom = Math.ceil(shape.bottom);
      this.#drawn.push(left, top, right, bottom);
      context.save();
      context.beginPath();
      context.rect(left, top, right - left, bottom - top);
      context.clip();
      const [r, g, b] = channels(shape.color).map((channel) => Math.round(channel * 255));
      const solid = `rgba(${r}, ${g}, ${b}, ${shape.alpha})`;
      let paint: string | CanvasGradient = solid;
      if (shape.fadePower > 0) {
        const gradient = context.createLinearGradient(
          shape.fadeFromX,
          shape.fadeFromY,
          shape.fadeToX,
          shape.fadeToY,
        );
        // A power curve, drawn as a few stops.
        for (const at of [0, 0.25, 0.5, 0.75, 1]) {
          gradient.addColorStop(
            at,
            `rgba(${r}, ${g}, ${b}, ${shape.alpha * at ** shape.fadePower})`,
          );
        }
        paint = gradient;
      }
      if (shape.haloRadius > 0) {
        context.shadowColor = `rgba(${r}, ${g}, ${b}, ${shape.alpha * shape.haloStrength})`;
        context.shadowBlur = shape.haloRadius * 2;
      }
      context.beginPath();
      switch (shape.kind) {
        case ShapeKind.Polygon:
          for (let corner = 0; corner < shape.count; corner++) {
            const x = shape.points[corner * 2] ?? 0;
            const y = shape.points[corner * 2 + 1] ?? 0;
            if (corner === 0) context.moveTo(x, y);
            else context.lineTo(x, y);
          }
          context.closePath();
          context.fillStyle = paint;
          context.fill();
          break;
        case ShapeKind.Disc:
          context.arc(shape.centreX, shape.centreY, shape.radius, 0, Math.PI * 2);
          context.fillStyle = paint;
          context.fill();
          break;
        case ShapeKind.Ring:
          context.arc(shape.centreX, shape.centreY, shape.radius, 0, Math.PI * 2);
          context.lineWidth = shape.ring;
          context.strokeStyle = paint;
          context.stroke();
          break;
        case ShapeKind.Box:
        case ShapeKind.BoxRing: {
          const x = shape.centreX - shape.halfWidth;
          const y = shape.centreY - shape.halfHeight;
          context.rect(x, y, shape.halfWidth * 2, shape.halfHeight * 2);
          if (shape.kind === ShapeKind.Box) {
            context.fillStyle = paint;
            context.fill();
          } else {
            context.lineWidth = shape.ring;
            context.strokeStyle = paint;
            context.stroke();
          }
          break;
        }
      }
      context.restore();
      if (shape.holeRight > shape.holeLeft && shape.holeBottom > shape.holeTop) {
        context.clearRect(
          shape.holeLeft,
          shape.holeTop,
          shape.holeRight - shape.holeLeft,
          shape.holeBottom - shape.holeTop,
        );
      }
    }
  }

  onLost(): void {
    // A 2D canvas is not taken away.
  }

  dispose(): void {
    this.clear();
  }
}
