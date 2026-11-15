/**
 * Numeric constants of the WebGPU interface.
 *
 * A browser provides these as global objects (`GPUBufferUsage` and the like). They are
 * written out here instead, with the values the WebGPU specification fixes, for two
 * reasons: the TypeScript DOM library declares their types but not the objects, and code
 * that names the globals cannot even be loaded where WebGPU does not exist.
 */

export const BUFFER_COPY_DST = 0x0008;
export const BUFFER_VERTEX = 0x0020;
export const BUFFER_UNIFORM = 0x0040;

export const TEXTURE_COPY_SRC = 0x01;
export const TEXTURE_COPY_DST = 0x02;
export const TEXTURE_BINDING = 0x04;

export const STAGE_VERTEX = 0x1;
export const STAGE_FRAGMENT = 0x2;
