/**
 * Settings for tests: every setting at the default the core resolves, with only what a
 * test cares about changed. The defaults are generated from the core's own, so a test
 * sees what a page is really told.
 */

import type { ConfigSettings } from "../src/core/types.ts";
import { DEFAULT_CONFIG } from "../src/wire/config.generated.ts";

/** The settings of each section a test changes, each section changed in part. */
export type SettingsChanges = {
  readonly [Section in keyof ConfigSettings]?: Partial<ConfigSettings[Section]>;
};

/** The default settings with `changes` made, section by section. */
export function settings(changes: SettingsChanges = {}): ConfigSettings {
  const made: Record<string, unknown> = { ...DEFAULT_CONFIG };
  for (const [section, values] of Object.entries(changes)) {
    const base = (DEFAULT_CONFIG as unknown as Record<string, unknown>)[section];
    made[section] =
      typeof base === "object" && base !== null && !Array.isArray(base)
        ? { ...base, ...(values as object) }
        : values;
  }
  return made as unknown as ConfigSettings;
}
