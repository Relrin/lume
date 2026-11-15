/**
 * How the instance records map onto WebGPU vertex buffers, as plain data.
 *
 * Keeping this apart from the code that calls WebGPU lets the mapping, and the alignment
 * rules WebGPU enforces on it, be checked without a GPU.
 */

import type { FieldFormat, RecordLayout } from "../layout.ts";

/** One vertex attribute, in the shape WebGPU takes. */
export interface VertexAttributePlan {
  shaderLocation: number;
  offset: number;
  format: FieldFormat;
}

/** A vertex buffer layout, in the shape WebGPU takes. */
export interface VertexBufferPlan {
  arrayStride: number;
  stepMode: "instance";
  attributes: VertexAttributePlan[];
}

/** The vertex buffer layout for a record layout: one record per instance. */
export function vertexBufferPlan(layout: RecordLayout): VertexBufferPlan {
  return {
    arrayStride: layout.stride,
    stepMode: "instance",
    attributes: layout.fields.map((field, shaderLocation) => ({
      shaderLocation,
      offset: field.offset,
      format: field.format,
    })),
  };
}
