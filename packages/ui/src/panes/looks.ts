/**
 * The look of one pane's terminal, where its profile asks for another than the window's.
 *
 * A profile may name a theme and a font size for the terminals started from it. Only
 * the terminal takes them: its colours, the colours it tells its program it has, and the
 * size of its text. The card around it stays the window's.
 *
 * The core resolves each theme a profile names and sends its tokens with the profiles,
 * so the page never resolves a theme itself. A theme the core could not find is left
 * out, and a pane that asks for it has the window's look.
 */

import { terminalAppearance } from "@lume/tokens";

import type { Profile, ProfileLook } from "../core/types.ts";
import type { Appearance } from "../theme.ts";
import { FONT_SIZE_MAX_PX, FONT_SIZE_MIN_PX } from "../tuning.ts";

/**
 * The looks of panes, each worked out once: the same profile gives the same object, so
 * a view that is given its look again can tell that nothing changed.
 */
export class PaneLooks {
  readonly #themed = new WeakMap<ProfileLook, Appearance>();
  readonly #sized = new WeakMap<Appearance, Map<number, Appearance>>();

  /**
   * The look a pane started from `profile` has. `looks` are the themes' looks as the
   * core sent them. The window's own look, unchanged, when the profile asks for nothing.
   */
  of(
    window: Appearance,
    profile: Profile | undefined,
    looks: Readonly<Record<string, ProfileLook>> | undefined,
  ): Appearance {
    const theme = profile?.theme === undefined ? undefined : looks?.[profile.theme];
    let base = window;
    if (theme) {
      base = this.#themed.get(theme) ?? terminalAppearance(theme.tokens, theme.scheme);
      this.#themed.set(theme, base);
    }
    const asked = profile?.font_size;
    if (asked === undefined) return base;
    const size = Math.min(Math.max(asked, FONT_SIZE_MIN_PX), FONT_SIZE_MAX_PX);
    if (size === base.fontSizePx) return base;
    let sizes = this.#sized.get(base);
    if (!sizes) {
      sizes = new Map();
      this.#sized.set(base, sizes);
    }
    let sized = sizes.get(size);
    if (!sized) {
      sized = { ...base, fontSizePx: size };
      sizes.set(size, sized);
    }
    return sized;
  }
}
