import { describe, expect, it } from "vitest";

import { TIER_ORDER } from "../src/constants.ts";
import { GLYPH, RECT } from "../src/layout.ts";
import { attributePlan } from "../src/webgl2/plan.ts";
import { GLYPH_FRAGMENT, GLYPH_VERTEX, RECT_FRAGMENT, RECT_VERTEX } from "../src/webgl2/shaders.ts";
import { vertexBufferPlan } from "../src/webgpu/plan.ts";
import { SHADER } from "../src/webgpu/shaders.ts";

describe("the tier order", () => {
  it("tries the GPU tiers first and ends with the one that always works", () => {
    expect([...TIER_ORDER]).toEqual(["webgpu", "webgl2", "canvas2d"]);
  });
});

describe("the WebGPU vertex plan", () => {
  it("gives one record per instance with the layout's offsets and formats", () => {
    expect(vertexBufferPlan(RECT)).toEqual({
      arrayStride: 12,
      stepMode: "instance",
      attributes: [
        { shaderLocation: 0, offset: 0, format: "sint16x2" },
        { shaderLocation: 1, offset: 4, format: "uint16x2" },
        { shaderLocation: 2, offset: 8, format: "unorm8x4" },
      ],
    });
    expect(vertexBufferPlan(GLYPH).attributes.map((a) => a.offset)).toEqual([0, 4, 8, 12, 16]);
  });

  it("obeys the alignment WebGPU validates", () => {
    for (const plan of [vertexBufferPlan(RECT), vertexBufferPlan(GLYPH)]) {
      // The record size and every offset must be multiples of four, and an attribute
      // must end inside the record.
      expect(plan.arrayStride % 4).toBe(0);
      for (const attribute of plan.attributes) {
        expect(attribute.offset % 4).toBe(0);
        expect(attribute.offset + 4).toBeLessThanOrEqual(plan.arrayStride);
      }
    }
  });

  it("matches the inputs the shader declares, location by location", () => {
    const wgslTypes: Record<string, string> = {
      sint16x2: "vec2i",
      uint16x2: "vec2u",
      unorm8x4: "vec4f",
    };
    for (const [entry, layout] of [
      ["rect_vertex", RECT],
      ["glyph_vertex", GLYPH],
    ] as const) {
      const signature = SHADER.slice(
        SHADER.indexOf(`fn ${entry}(`),
        SHADER.indexOf(`) ->`, SHADER.indexOf(`fn ${entry}(`)),
      );
      for (const attribute of vertexBufferPlan(layout).attributes) {
        const field = layout.fields[attribute.shaderLocation]!;
        expect(signature, `${entry} ${field.name}`).toContain(
          `@location(${attribute.shaderLocation}) ${field.name}: ${wgslTypes[attribute.format]}`,
        );
      }
    }
  });
});

describe("the WebGL2 attribute plan", () => {
  it("reads positions and sizes as integers and colours as fractions", () => {
    expect(attributePlan(RECT)).toEqual({
      stride: 12,
      attributes: [
        { location: 0, offset: 0, components: 2, type: "short", integer: true },
        { location: 1, offset: 4, components: 2, type: "unsigned-short", integer: true },
        { location: 2, offset: 8, components: 4, type: "unsigned-byte", integer: false },
      ],
    });
  });

  it("matches the inputs the shaders declare, location by location", () => {
    const glslTypes = (integer: boolean, type: string, components: number) =>
      integer ? (type === "short" ? `ivec${components}` : `uvec${components}`) : `vec${components}`;
    for (const [source, layout] of [
      [RECT_VERTEX, RECT],
      [GLYPH_VERTEX, GLYPH],
    ] as const) {
      for (const attribute of attributePlan(layout).attributes) {
        const field = layout.fields[attribute.location]!;
        const type = glslTypes(attribute.integer, attribute.type, attribute.components);
        expect(source, field.name).toContain(
          `layout(location = ${attribute.location}) in ${type} a_${field.name};`,
        );
      }
    }
  });

  it("writes premultiplied colour from both fragment shaders", () => {
    expect(RECT_FRAGMENT).toContain("v_color.rgb * v_color.a");
    expect(GLYPH_FRAGMENT).toContain("v_color.rgb * coverage");
    expect(GLYPH_FRAGMENT).toContain("texel.rgb * texel.a");
  });
});
