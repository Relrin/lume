// The prelude: what a plugin's code finds before it runs. It builds the `lume` object
// and `console`, and gives the runtime the one function it calls the code with.
//
// It is evaluated as a function and called with `call`, the runtime's only way in, and
// `info`, what the plugin is. Neither is left anywhere the plugin's code can reach.
// `call(id, name, json)` sends a call and says whether it was taken; id 0 asks for no
// answer. The function returned is `dispatch(kind, id, json)`.
(function (call, info) {
  "use strict";
  const stringify = JSON.stringify;
  const parse = JSON.parse;
  const freeze = Object.freeze;

  /** A call that failed, with the kind of failure in `code`. */
  class LumeError extends Error {
    constructor(code, message) {
      super(message);
      this.name = "LumeError";
      this.code = code;
    }
  }

  const pending = new Map();
  let nextCall = 1;
  const handlers = new Map();
  const timers = new Map();
  let nextTimer = 1;
  const listeners = new Map();
  const leaving = [];

  function text(value) {
    if (typeof value === "string") return value;
    if (value instanceof Error)
      return value.stack ? `${value.message}\n${value.stack}` : String(value);
    try {
      return stringify(value) ?? String(value);
    } catch {
      return String(value);
    }
  }

  function tell(name, args) {
    call(0, name, stringify(args ?? {}));
  }

  function log(level, values) {
    tell("log.write", { level, message: values.map(text).join(" ") });
  }

  function request(name, args) {
    return new Promise((resolve, reject) => {
      const id = nextCall++;
      pending.set(id, { resolve, reject });
      let taken;
      try {
        taken = call(id, name, stringify(args ?? {}));
      } catch (error) {
        pending.delete(id);
        reject(error);
        return;
      }
      if (!taken) {
        pending.delete(id);
        reject(new LumeError("busy", "the plugin is making too many calls at once"));
      }
    });
  }

  function failure(error) {
    if (error instanceof LumeError) return { code: error.code, message: error.message };
    return { code: "failed", message: text(error) };
  }

  function listen(topic, listener, subscribe) {
    if (typeof listener !== "function") throw new TypeError("a listener is a function");
    let set = listeners.get(topic);
    if (!set) {
      set = new Set();
      listeners.set(topic, set);
      if (subscribe) {
        request("events.subscribe", { topic }).catch((error) => log("error", [error]));
      }
    }
    set.add(listener);
    return freeze({ dispose: () => set.delete(listener) });
  }

  function timer(repeat, fn, ms, args) {
    if (typeof fn !== "function") throw new TypeError("a timer calls a function");
    const id = nextTimer++;
    timers.set(id, { fn, args, repeat });
    tell("timers.set", { id, ms: Math.max(0, Number(ms) || 0), repeat });
    return id;
  }

  function clearTimer(id) {
    if (timers.delete(id)) tell("timers.clear", { id });
  }

  const readings = new Map();

  function subscribeMetrics(kinds, listener, options) {
    if (typeof listener !== "function") throw new TypeError("a listener is a function");
    const wanted = [...(kinds ?? [])].map(String);
    const visible = [...(options?.while ?? [])].map(String);
    const held = { kinds: wanted, listener, id: null, gone: false };
    request("metrics.subscribe", { kinds: wanted, visible }).then(
      (id) => {
        if (held.gone) request("metrics.unsubscribe", { subscription: id }).catch(() => {});
        else {
          held.id = id;
          readings.set(id, held);
        }
      },
      (error) => log("error", [error]),
    );
    return freeze({
      dispose: () => {
        held.gone = true;
        if (held.id === null) return;
        readings.delete(held.id);
        request("metrics.unsubscribe", { subscription: held.id }).catch(() => {});
      },
    });
  }

  const lume = {
    self: freeze({ ...info }),
    commands: {
      /** Carries out one of the plugin's commands, by the name its manifest gives it. */
      register(name, handler) {
        if (typeof handler !== "function") throw new TypeError("a command's handler is a function");
        handlers.set(String(name), handler);
        return freeze({ dispose: () => handlers.delete(String(name)) });
      },
    },
    timers: {
      setTimeout: (fn, ms, ...args) => timer(false, fn, ms, args),
      setInterval: (fn, ms, ...args) => timer(true, fn, ms, args),
      clearTimeout: clearTimer,
      clearInterval: clearTimer,
    },
    sessions: {
      onEvent: (listener) => listen("session", listener, true),
    },
    settings: {
      onChange: (listener) => listen("settings", listener, false),
    },
    metrics: {
      subscribe: subscribeMetrics,
    },
    log: {
      debug: (...values) => log("debug", values),
      info: (...values) => log("info", values),
      warn: (...values) => log("warn", values),
      error: (...values) => log("error", values),
    },
    onDeactivate(fn) {
      if (typeof fn !== "function") throw new TypeError("onDeactivate takes a function");
      leaving.push(fn);
    },
    LumeError,
  };

  function namespace(path) {
    let at = lume;
    for (const part of path) {
      at[part] = at[part] ?? {};
      at = at[part];
    }
    return at;
  }

  /*METHODS*/

  function freezeAll(value) {
    if (value && typeof value === "object" && !Object.isFrozen(value)) {
      freeze(value);
      for (const key of Object.keys(value)) freezeAll(value[key]);
    }
    return value;
  }

  Object.defineProperty(globalThis, "lume", { value: freezeAll(lume), enumerable: true });
  Object.defineProperty(globalThis, "console", {
    value: freeze({
      log: (...values) => log("info", values),
      info: (...values) => log("info", values),
      debug: (...values) => log("debug", values),
      warn: (...values) => log("warn", values),
      error: (...values) => log("error", values),
    }),
    enumerable: true,
  });

  function report(error) {
    log("error", [error]);
  }

  return function dispatch(kind, id, json) {
    const data = json === undefined || json === "" ? undefined : parse(json);
    switch (kind) {
      case "result": {
        const waiting = pending.get(id);
        if (waiting) {
          pending.delete(id);
          waiting.resolve(data);
        }
        return;
      }
      case "error": {
        const waiting = pending.get(id);
        if (waiting) {
          pending.delete(id);
          waiting.reject(new LumeError(data.code, data.message));
        }
        return;
      }
      case "timer": {
        const due = timers.get(id);
        if (!due) return;
        if (!due.repeat) timers.delete(id);
        try {
          due.fn(...due.args);
        } catch (error) {
          report(error);
        }
        return;
      }
      case "command": {
        const handler = handlers.get(data.name);
        if (!handler) {
          tell("commands.done", {
            invocation: id,
            error: { code: "unhandled", message: `the plugin has no handler for ${data.name}` },
          });
          return;
        }
        Promise.resolve()
          .then(() => handler(data.args ?? null))
          .then(
            (value) =>
              tell("commands.done", { invocation: id, value: value === undefined ? null : value }),
            (error) => {
              report(error);
              tell("commands.done", { invocation: id, error: failure(error) });
            },
          );
        return;
      }
      case "event": {
        if (data.topic === "metrics") {
          const reading = data.payload.sample ?? {};
          for (const id of data.payload.to ?? []) {
            const held = readings.get(id);
            if (!held) continue;
            const sample = {};
            for (const kind of held.kinds) {
              if (reading[kind] !== undefined) sample[kind] = reading[kind];
            }
            try {
              held.listener(freeze(sample));
            } catch (error) {
              report(error);
            }
          }
          return;
        }
        const set = listeners.get(data.topic);
        if (!set) return;
        for (const listener of [...set]) {
          try {
            listener(data.payload);
          } catch (error) {
            report(error);
          }
        }
        return;
      }
      case "leave": {
        for (const fn of leaving.splice(0)) {
          try {
            fn();
          } catch (error) {
            report(error);
          }
        }
        for (const waiting of pending.values()) {
          waiting.reject(new LumeError("stopped", "the plugin was stopped"));
        }
        pending.clear();
        timers.clear();
        listeners.clear();
        handlers.clear();
        readings.clear();
        return;
      }
      default:
        return;
    }
  };
});
