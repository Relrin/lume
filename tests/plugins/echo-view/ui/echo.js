// The echo view's page. It says how it was started, keeps a count that goes up with
// every start, and then does the task its state names: `streams` uses its plugin's
// streams in every way a view may and may not, and `bench` times them.

"use strict";

let port = null;
let hello = null;
let nextId = 1;
const waiting = new Map();
const listeners = new Map();
const lines = [];

function post(message, transfer) {
  port.postMessage(message, transfer ?? []);
}

function ask(message) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    post({ ...message, id });
  });
}

const call = (name, args) => ask({ t: "call", name, args: args ?? null });

function on(event, listener) {
  listeners.set(event, listener);
}

function say(line) {
  lines.push(line);
  document.getElementById("state").textContent = line;
}

async function report() {
  await call("report", { lines: lines.splice(0) });
}

// A stream the plugin's code let this view use, by its grant: what arrives is handed
// to `piece` and acknowledged, which lets more come.
async function openStream(grant, piece) {
  const opened = await ask({ t: "stream", grant });
  const end = opened.port;
  let finish;
  const ended = new Promise((resolve) => (finish = resolve));
  let bytes = 0;
  end.onmessage = (event) => {
    const data = event.data;
    if (data.d) {
      bytes += data.d.byteLength;
      if (piece) piece(new Uint8Array(data.d));
      end.postMessage({ ack: data.d.byteLength });
    } else if (data.end) {
      finish({ ok: data.ok, bytes });
    }
  };
  return {
    readable: opened.readable,
    writable: opened.writable,
    ended,
    write(text) {
      const buffer = new TextEncoder().encode(text).buffer;
      end.postMessage({ w: buffer }, [buffer]);
    },
    finish: () => end.postMessage({ fin: true }),
    close: () => end.postMessage({ close: true }),
  };
}

const refusal = (error) => String(error.code ?? error.message);

async function streams() {
  // A failure of the plugin's own kind arrives as its code named it.
  const own = await call("fail").then(
    () => "answered",
    (error) => `${error.code} ${JSON.stringify(error.data)} ${error.message}`,
  );
  say(`own ${own}`);

  // A program's input written and its output read, without the plugin's code between.
  const sort = await call("sort");
  let sorted = "";
  const decoder = new TextDecoder();
  const output = await openStream(sort.output, (bytes) => {
    sorted += decoder.decode(bytes, { stream: true });
  });
  const input = await openStream(sort.input);
  input.write("b\nc\n");
  input.write("a\n");
  input.finish();
  const done = await output.ended;
  say(`sorted ${sorted.trim().split(/\r?\n/).join(",")} ended ${done.ok}`);
  say(
    `ways output ${output.readable}/${output.writable} input ${input.readable}/${input.writable}`,
  );

  // A grant is opened once.
  const again = await openStream(sort.output).then(
    () => "opened",
    (error) => `refused ${refusal(error)}`,
  );
  say(`reopen ${again}`);

  // A grant made for another view of the same plugin is none here.
  const others = (await call("instances")).filter((one) => one.instance !== hello.instance);
  if (others.length > 0) {
    const foreign = await call("grant-for", { instance: others[0].instance });
    const tried = await openStream(foreign.grant).then(
      () => "opened",
      (error) => `refused ${refusal(error)}`,
    );
    say(`foreign ${tried}`);
  } else {
    say("foreign untried");
  }

  // Nor is one that was never given.
  const guessed = await openStream("0123456789abcdef0123456789abcdef").then(
    () => "opened",
    (error) => `refused ${refusal(error)}`,
  );
  say(`guessed ${guessed}`);

  // A grant taken back ends its stream: the file is far larger than what a view is
  // sent before it says it took some, so the stream cannot have run out by itself.
  const file = await call("file", { bytes: 4 * 1024 * 1024 });
  const opened = await ask({ t: "stream", grant: file.grant });
  const outcome = new Promise((resolve) => {
    opened.port.onmessage = (event) => {
      if (event.data.end) resolve(`ended ${event.data.ok}`);
    };
  });
  await call("revoke", { grant: file.grant });
  say(`revoked ${await outcome}`);

  await call("spare");
  say("done");
}

async function timeFile(bytes, write) {
  const file = await call("file", { bytes, write });
  const began = performance.now();
  const stream = await openStream(file.grant);
  const done = await stream.ended;
  return { bytes: done.bytes, ms: performance.now() - began };
}

function timeRelay() {
  return new Promise((resolve) => {
    const began = performance.now();
    let bytes = 0;
    on("piece", (piece) => {
      bytes += atob(piece).length;
    });
    on("end", () => resolve({ bytes, ms: performance.now() - began }));
    call("relay");
  });
}

async function timeSort() {
  const sort = await call("sort");
  const output = await openStream(sort.output);
  const input = await openStream(sort.input);
  const began = performance.now();
  input.write("b\na\n".repeat(128));
  input.finish();
  await output.ended;
  return performance.now() - began;
}

const mib = (timed) => timed.bytes / (1024 * 1024) / (timed.ms / 1000);

function quantile(samples, q) {
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];
}

async function bench(size) {
  const direct = [];
  const relayed = [];
  for (let run = 0; run < 3; run++) {
    direct.push(mib(await timeFile(size, run === 0)));
    relayed.push(mib(await timeRelay()));
  }
  const trips = [];
  for (let run = 0; run < 15; run++) trips.push(await timeSort());
  say(
    `bench ${JSON.stringify({
      bytes: size,
      streamMibPerSecond: quantile(direct, 0.5),
      relayMibPerSecond: quantile(relayed, 0.5),
      sortRoundTripMs: { p50: quantile(trips, 0.5), p95: quantile(trips, 0.95) },
    })}`,
  );
}

async function start() {
  const state = hello.state ?? {};
  const count = typeof state.count === "number" ? state.count : 0;
  const opens = state.open ? ` open ${state.open.path ?? state.open.uri}` : "";
  say(`started ${hello.place} count ${count}${opens}`);
  // What is kept is found again the next time the view starts.
  post({ t: "state", state: { ...state, count: count + 1, task: null } });
  await report();
  if (state.task === "streams") await streams();
  if (state.task === "bench") await bench(state.bytes ?? 16 * 1024 * 1024);
  // One long read of a stream, for whoever times the window meanwhile.
  if (state.task === "flow") {
    say(`flow ${mib(await timeFile(state.bytes ?? 64 * 1024 * 1024, true)).toFixed(1)}`);
  }
  await report();
}

// Keys this page has no use for are the window's: the ones held with Ctrl, Alt or the
// logo key, and the function keys.
window.addEventListener("keydown", (event) => {
  if (port === null || event.defaultPrevented) return;
  const mods = [];
  if (event.ctrlKey) mods.push("ctrl");
  if (event.altKey) mods.push("alt");
  if (event.shiftKey) mods.push("shift");
  if (event.metaKey) mods.push("meta");
  const held = event.ctrlKey || event.altKey || event.metaKey;
  if (!held && !/^F\d+$/.test(event.key)) return;
  post({ t: "key", key: event.key, code: event.code, mods });
});

window.addEventListener("message", (event) => {
  if (port !== null || !event.data || event.data.lume !== "connect" || event.ports.length !== 1)
    return;
  hello = event.data;
  port = event.ports[0];
  port.onmessage = (message) => {
    const data = message.data;
    if (data.t === "event") {
      listeners.get(data.event)?.(data.payload);
      return;
    }
    if (data.t === "focus") {
      document.getElementById("field").focus();
      return;
    }
    const found = waiting.get(data.id);
    if (!found) return;
    waiting.delete(data.id);
    if (data.t === "result") found.resolve(data.value);
    else if (data.t === "opened") found.resolve(data);
    else found.reject(Object.assign(new Error(data.message), { code: data.code, data: data.data }));
  };
  post({ t: "ready" });
  start().catch((error) => {
    say(`failed ${error.code ?? ""} ${error.message}`);
    report();
  });
});
