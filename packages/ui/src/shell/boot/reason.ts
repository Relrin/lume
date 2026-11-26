/** What went wrong, in words, whatever was thrown. */
export function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
