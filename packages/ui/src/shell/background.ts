/**
 * Telling what kind of picture a file is from its first bytes.
 *
 * The picture behind the panes reaches the page as bytes with no name, and the page
 * shows it only if it starts the way one of the kinds of picture it draws starts.
 * Nothing here touches the page, so the rule is tested with plain bytes.
 */

/** The kinds of picture the page shows, as a browser names them. */
export type ImageType = "image/png" | "image/jpeg" | "image/webp" | "image/gif";

function startsWith(bytes: Uint8Array, at: number, expected: readonly number[]): boolean {
  if (bytes.length < at + expected.length) return false;
  return expected.every((byte, index) => bytes[at + index] === byte);
}

const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG = [0xff, 0xd8, 0xff];
const RIFF = [0x52, 0x49, 0x46, 0x46];
const WEBP = [0x57, 0x45, 0x42, 0x50];
const GIF87 = [0x47, 0x49, 0x46, 0x38, 0x37, 0x61];
const GIF89 = [0x47, 0x49, 0x46, 0x38, 0x39, 0x61];

/** The kind of picture `bytes` hold, or nothing when they hold none the page shows. */
export function imageType(bytes: Uint8Array): ImageType | undefined {
  if (startsWith(bytes, 0, PNG)) return "image/png";
  if (startsWith(bytes, 0, JPEG)) return "image/jpeg";
  // A WebP picture is a RIFF file: the four letters follow the file's length.
  if (startsWith(bytes, 0, RIFF) && startsWith(bytes, 8, WEBP)) return "image/webp";
  if (startsWith(bytes, 0, GIF87) || startsWith(bytes, 0, GIF89)) return "image/gif";
  return undefined;
}
