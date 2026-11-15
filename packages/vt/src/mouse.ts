/**
 * Encoding mouse events as the bytes a program expects.
 *
 * A program chooses which events it wants (presses only, drags, all motion) and in which
 * of several formats. The encoder takes both from the terminal. It works from pixel
 * positions, because one of the formats reports pixels, and converts to cells itself
 * from the geometry it has been given.
 */

import type { Core, Vt } from "./module.ts";
import type { Terminal } from "./terminal.ts";

/** What the mouse did. */
export type MouseAction = "press" | "release" | "motion";

/** A mouse button. Buttons four and five are how wheel steps up and down are reported. */
export type MouseButton =
  | "left"
  | "right"
  | "middle"
  | "four"
  | "five"
  | "six"
  | "seven"
  | "eight"
  | "nine"
  | "ten"
  | "eleven";

/** The surface the terminal is drawn on, in pixels. */
export interface MouseGeometry {
  screenWidth: number;
  screenHeight: number;
  cellWidth: number;
  cellHeight: number;
  paddingTop?: number;
  paddingBottom?: number;
  paddingRight?: number;
  paddingLeft?: number;
}

/** A mouse event to encode. */
export interface MouseInput {
  action: MouseAction;
  /** The button concerned. Leave it out for motion with no button held. */
  button?: MouseButton;
  /** `Mod` bits held during the event. */
  mods?: number;
  /** Position in pixels from the top-left corner of the surface. */
  x: number;
  y: number;
  /** Whether any button is down, which some tracking modes need for motion. */
  anyButtonPressed?: boolean;
}

/** Encodes mouse events for a terminal. One is enough for a pane. */
export class MouseEncoder {
  readonly #core: Core;
  #encoder: number;
  #event: number;
  readonly #actions: Readonly<Record<MouseAction, number>>;
  readonly #buttons: Readonly<Record<MouseButton, number>>;
  readonly #option: Readonly<Record<string, number>>;
  /** The terminal and revision the tracking mode and format were last read from. */
  #syncedWith: Terminal | undefined;
  #syncedRevision = -1;
  #geometry: MouseGeometry | undefined;

  constructor(vt: Vt) {
    const core = vt.core;
    this.#core = core;
    const m = core.manifest;
    const button = (name: string) => m.enumValue("GhosttyMouseButton", name);

    this.#actions = {
      press: m.enumValue("GhosttyMouseAction", "PRESS"),
      release: m.enumValue("GhosttyMouseAction", "RELEASE"),
      motion: m.enumValue("GhosttyMouseAction", "MOTION"),
    };
    this.#buttons = {
      left: button("LEFT"),
      right: button("RIGHT"),
      middle: button("MIDDLE"),
      four: button("FOUR"),
      five: button("FIVE"),
      six: button("SIX"),
      seven: button("SEVEN"),
      eight: button("EIGHT"),
      nine: button("NINE"),
      ten: button("TEN"),
      eleven: button("ELEVEN"),
    };
    this.#option = m.enumValues("GhosttyMouseEncoderOption");

    this.#encoder = core.newHandle("ghostty_mouse_encoder_new", (out) =>
      core.x.ghostty_mouse_encoder_new(0, out),
    );
    this.#event = 0;
    try {
      this.#event = core.newHandle("ghostty_mouse_event_new", (out) =>
        core.x.ghostty_mouse_event_new(0, out),
      );
    } catch (err) {
      this.free();
      throw err;
    }
  }

  /** Describe the surface. Needed before the first event; call again when it changes. */
  setGeometry(geometry: MouseGeometry): void {
    this.#geometry = { ...geometry };
    this.#applyGeometry();
  }

  /**
   * The bytes to send for a mouse event under the terminal's current mouse modes. Empty
   * when the program has not asked for the event, or when motion stayed within one cell.
   */
  encode(terminal: Terminal, input: MouseInput): Uint8Array {
    if (this.#encoder === 0) throw new Error("this mouse encoder has been freed");
    if (!this.#geometry) throw new Error("the mouse encoder has no geometry yet");
    const core = this.#core;
    const { x } = core;
    const event = this.#event;

    // Reading the modes from the terminal also forgets the last cell reported, which
    // would make every motion event look new. So it is done only when the terminal has
    // processed something since.
    if (this.#syncedWith !== terminal || this.#syncedRevision !== terminal.revision) {
      core.check(
        x.ghostty_mouse_encoder_set_from_terminal(this.#encoder, terminal.handle),
        "ghostty_mouse_encoder_set_from_terminal",
      );
      this.#syncedWith = terminal;
      this.#syncedRevision = terminal.revision;
      // Report motion once per cell, not once per pixel.
      this.#setBool("TRACK_LAST_CELL", true);
    }
    this.#setBool("ANY_BUTTON_PRESSED", input.anyButtonPressed ?? false);

    core.check(x.ghostty_mouse_event_set_action(event, this.#actions[input.action]), "set_action");
    if (input.button === undefined) {
      core.check(x.ghostty_mouse_event_clear_button(event), "clear_button");
    } else {
      core.check(
        x.ghostty_mouse_event_set_button(event, this.#buttons[input.button]),
        "set_button",
      );
    }
    core.check(x.ghostty_mouse_event_set_mods(event, input.mods ?? 0), "set_mods");

    // The position is passed by value in C; the export takes the address of a copy.
    const m = core.manifest;
    const position = core.scratch(m.struct("GhosttyMousePosition").size);
    const view = core.view();
    view.setFloat32(position + m.offset("GhosttyMousePosition", "x"), input.x, true);
    view.setFloat32(position + m.offset("GhosttyMousePosition", "y"), input.y, true);
    core.check(x.ghostty_mouse_event_set_position(event, position), "set_position");

    return core.encode("ghostty_mouse_encoder_encode", (buf, cap, written) =>
      x.ghostty_mouse_encoder_encode(this.#encoder, event, buf, cap, written),
    );
  }

  /** Release the encoder. The object must not be used afterwards. */
  free(): void {
    const { x } = this.#core;
    if (this.#event !== 0) x.ghostty_mouse_event_free(this.#event);
    if (this.#encoder !== 0) x.ghostty_mouse_encoder_free(this.#encoder);
    this.#event = 0;
    this.#encoder = 0;
    this.#syncedWith = undefined;
  }

  #applyGeometry(): void {
    const geometry = this.#geometry;
    if (!geometry || this.#encoder === 0) return;
    const core = this.#core;
    const m = core.manifest;
    const layout = m.struct("GhosttyMouseEncoderSize");
    const at = core.scratch(layout.size);
    core.x.ghostty_sized_init(at, layout.size);
    const view = core.view();
    const set = (field: string, value: number) =>
      view.setUint32(at + m.offset("GhosttyMouseEncoderSize", field), Math.round(value), true);
    set("screen_width", geometry.screenWidth);
    set("screen_height", geometry.screenHeight);
    set("cell_width", geometry.cellWidth);
    set("cell_height", geometry.cellHeight);
    set("padding_top", geometry.paddingTop ?? 0);
    set("padding_bottom", geometry.paddingBottom ?? 0);
    set("padding_right", geometry.paddingRight ?? 0);
    set("padding_left", geometry.paddingLeft ?? 0);
    this.#set("SIZE", at);
  }

  #setBool(option: string, value: boolean): void {
    const at = this.#core.scratch(4);
    this.#core.view().setUint8(at, value ? 1 : 0);
    this.#set(option, at);
  }

  #set(option: string, value: number): void {
    const key = this.#option[option];
    if (key === undefined) throw new Error(`libghostty-vt has no mouse encoder option ${option}`);
    this.#core.check(
      this.#core.x.ghostty_mouse_encoder_set(this.#encoder, key, value),
      `ghostty_mouse_encoder_set(${option})`,
    );
  }
}
