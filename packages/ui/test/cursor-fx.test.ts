import type { CursorEffectSettings, FxRect } from "@lume/renderer";
import { describe, expect, it } from "vitest";

import {
  CursorFx,
  cursorFxSettings,
  type CursorReport,
  type CursorSource,
  type EffectsLike,
} from "../src/terminal/cursor-fx.ts";
import { view } from "./settings-support.ts";

class FakeEffects implements EffectsLike {
  settings: CursorEffectSettings | undefined;
  moves: { from: FxRect; to: FxRect }[] = [];
  pulses: FxRect[] = [];
  cancelled = 0;
  disposed = false;
  configure(settings: CursorEffectSettings) {
    this.settings = settings;
  }
  resize() {}
  move(from: FxRect, to: FxRect) {
    this.moves.push({ from, to });
    return true;
  }
  pulse(at: FxRect) {
    this.pulses.push(at);
    return true;
  }
  cancel() {
    this.cancelled++;
  }
  dispose() {
    this.disposed = true;
  }
}

/** A pane whose cells are 10 by 20 pixels, its top left at `left`, `top` in the window. */
function pane(left: number, top = 0): CursorSource {
  return {
    cellRect: (col, row) => ({ x: left + col * 10, y: top + row * 20, width: 10, height: 20 }),
  };
}

function at(col: number, row: number, more: Partial<CursorReport> = {}): CursorReport {
  return {
    col,
    row,
    shape: 1,
    visible: true,
    focused: true,
    reflowed: false,
    color: 0xffb8e37f,
    ...more,
  };
}

function setup(reduced = { matches: false }) {
  const effects = new FakeEffects();
  const fx = new CursorFx(() => effects, reduced);
  fx.configure({ trail: "comet", pulse: "ripple" });
  fx.attach({} as HTMLElement);
  return { fx, effects };
}

describe("the cursor's effects in a window", () => {
  it("stop telling of an effect that cannot be drawn once told to stop", () => {
    const told: string[] = [];
    let tell: ((id: string, message: string) => void) | undefined;
    const effects = Object.assign(new FakeEffects(), {
      onProblem: (listener: (id: string, message: string) => void) => void (tell = listener),
    });
    const fx = new CursorFx(() => effects, { matches: false });
    const stop = fx.onProblem((id) => told.push(id));
    fx.attach({} as HTMLElement);
    tell?.("acme/glow", "no shader");
    stop();
    tell?.("acme/glow", "no shader");
    // Effects made again for another element are not told of the listener either.
    fx.attach({} as HTMLElement);
    tell?.("acme/glow", "no shader");
    expect(told).toEqual(["acme/glow"]);
  });

  it("say nothing until the first cursor is known, then follow its jumps", () => {
    const { fx, effects } = setup();
    const left = pane(0);
    fx.report(left, at(0, 0), 0);
    expect(effects.moves).toEqual([]);
    fx.report(left, at(0, 0), 16);
    expect(effects.moves).toEqual([]);
    fx.report(left, at(40, 10), 32);
    expect(effects.moves).toEqual([
      {
        from: { x: 0, y: 0, width: 10, height: 20 },
        to: { x: 400, y: 200, width: 10, height: 20 },
      },
    ]);
  });

  it("fly from one pane's cursor to the other's when the keyboard moves, with a pulse", () => {
    const { fx, effects } = setup();
    const left = pane(0);
    const right = pane(600);
    fx.report(left, at(3, 2), 0);
    // The pane that loses the keyboard says so, and is still where the trail leaves from.
    fx.report(left, at(3, 2, { focused: false }), 16);
    fx.report(right, at(0, 0), 16);
    expect(effects.moves).toEqual([
      {
        from: { x: 30, y: 40, width: 10, height: 20 },
        to: { x: 600, y: 0, width: 10, height: 20 },
      },
    ]);
    expect(effects.pulses).toEqual([{ x: 600, y: 0, width: 10, height: 20 }]);
  });

  it("play nothing for a cursor the grid moved, or one that is hidden or scrolled away", () => {
    const { fx, effects } = setup();
    const only = pane(0);
    fx.report(only, at(0, 5), 0);
    fx.report(only, at(0, 30, { reflowed: true }), 16);
    fx.report(only, at(20, 0, { visible: false }), 32);
    expect(effects.moves).toEqual([]);
    // The cursor shown again is measured from where it last was.
    fx.report(only, at(0, 30), 48);
    expect(effects.moves.length).toBe(0);
  });

  it("pulse when the cursor changes shape where it is", () => {
    const { fx, effects } = setup();
    const only = pane(0);
    fx.report(only, at(4, 4), 0);
    fx.report(only, at(4, 4, { shape: 0 }), 16);
    expect(effects.moves).toEqual([]);
    expect(effects.pulses).toEqual([{ x: 40, y: 80, width: 10, height: 20 }]);
  });

  it("start again from nothing when the pane on show goes", () => {
    const { fx, effects } = setup();
    const old = pane(0);
    fx.report(old, at(0, 0), 0);
    fx.forget(old);
    fx.report(pane(600), at(0, 0), 16);
    expect(effects.moves).toEqual([]);
    expect(effects.pulses).toEqual([]);
  });

  it("stay off while every effect is, and while the system asks for less motion", () => {
    const reduced = { matches: false };
    const { fx, effects } = setup(reduced);
    expect(fx.watching).toBe(true);
    reduced.matches = true;
    fx.configure();
    expect(fx.watching).toBe(false);
    expect(effects.settings).toEqual({ trail: "none", pulse: "none" });
    expect(effects.cancelled).toBe(1);
    reduced.matches = false;
    fx.configure();
    expect(effects.settings).toEqual({ trail: "comet", pulse: "ripple" });
    fx.configure({ trail: "none", pulse: "none" });
    expect(fx.watching).toBe(false);
    fx.detach();
    expect(effects.disposed).toBe(true);
  });

  it("read what the configuration asks for, and nothing from a core that does not say", () => {
    expect(cursorFxSettings(null)).toEqual({ trail: "none", pulse: "none" });
    const asked = view();
    const terminal = {
      ...asked.config.terminal,
      "cursor-trail": "warp" as const,
      "cursor-pulse": "box-burst" as const,
      "cursor-effect-length": 1.5,
    };
    expect(cursorFxSettings({ ...asked, config: { ...asked.config, terminal } })).toEqual({
      trail: "warp",
      pulse: "box-burst",
      length: 1.5,
    });
  });
});
