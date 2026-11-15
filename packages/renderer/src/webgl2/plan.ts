/**
 * How the instance records map onto WebGL2 vertex attributes, as plain data.
 *
 * Keeping this apart from the code that calls WebGL lets the mapping be checked without
 * a GPU.
 */

import type { FieldFormat, RecordLayout } from "../layout.ts";

/** The type of one component of an attribute. */
export type ComponentType = "short" | "unsigned-short" | "unsigned-byte";

/** One vertex attribute. */
export interface AttributePlan {
  /** The `layout(location = n)` of the attribute in the shader. */
  location: number;
  components: number;
  type: ComponentType;
  /**
   * Whether the shader reads it as integers. Integer attributes are set up with a
   * different call than floating-point ones, and mixing them up reads garbage.
   */
  integer: boolean;
  offset: number;
}

const FORMATS: Readonly<Record<FieldFormat, Omit<AttributePlan, "location" | "offset">>> = {
  sint16x2: { components: 2, type: "short", integer: true },
  uint16x2: { components: 2, type: "unsigned-short", integer: true },
  // Colour bytes become fractions from 0 to 1.
  unorm8x4: { components: 4, type: "unsigned-byte", integer: false },
};

/** The attributes of a record layout, in field order, and the record size. */
export function attributePlan(layout: RecordLayout): {
  stride: number;
  attributes: AttributePlan[];
} {
  return {
    stride: layout.stride,
    attributes: layout.fields.map((field, location) => ({
      location,
      offset: field.offset,
      ...FORMATS[field.format],
    })),
  };
}
