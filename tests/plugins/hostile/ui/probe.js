// The hostile view's page. It waits for the channel the window hands it, then tries one
// way out after another and reports, for each, whether it got anywhere. "reached" means
// the try got past what should have stopped it; "blocked" means it did not.
//
// The tries that would talk to the network aim at a listener the test runs on this
// machine, which counts every connection it is given.

"use strict";

const results = [];
let port = null;
let nextCall = 1;
const waiting = new Map();

function record(id, reached, detail) {
  results.push({
    id,
    outcome: reached ? "reached" : "blocked",
    detail: String(detail).slice(0, 300),
  });
}

// Something that is there but is not a way out by itself; what it leads to is tried and
// recorded on its own.
function note(id, detail) {
  results.push({ id, outcome: "noted", detail: String(detail).slice(0, 300) });
}

// Whether a connection a page opens is opened, or fails, within `ms`.
function connects(made, ms = 1500) {
  return new Promise((resolve) => {
    const end = (opened) => {
      try {
        made.close();
      } catch {
        // Closed already.
      }
      resolve(opened);
    };
    made.addEventListener("open", () => end(true));
    made.addEventListener("error", () => end(false));
    setTimeout(() => end(false), ms);
  });
}

// Whether a document put in a nested frame runs a script, within `ms`.
function nestedRuns(setUp, ms = 1500) {
  return new Promise((resolve) => {
    const frame = document.createElement("iframe");
    const said = (event) => {
      if (event.data === "nested ran") end(true);
    };
    const end = (ran) => {
      window.removeEventListener("message", said);
      frame.remove();
      resolve(ran);
    };
    window.addEventListener("message", said);
    setUp(frame);
    document.body.append(frame);
    setTimeout(() => end(false), ms);
  });
}

function ask(message) {
  const id = nextCall++;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    port.postMessage({ ...message, id });
  });
}

function call(name, args) {
  return ask({ t: "call", name, args });
}

function settle(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// Gives what a try did: whether it threw, and what it gave or threw. A try that never
// settles counts as having thrown after two seconds: it got no answer.
async function attempt(fn, ms = 2000) {
  try {
    const value = await Promise.race([
      fn(),
      settle(ms).then(() => {
        throw new Error("no answer");
      }),
    ]);
    return { threw: false, value };
  } catch (error) {
    return {
      threw: true,
      value: error && error.name ? `${error.name}: ${error.message}` : String(error),
    };
  }
}

// Loads something by an element and says whether it loaded, within `ms`.
function loads(tag, attributes, ms = 1500) {
  return new Promise((resolve) => {
    const element = document.createElement(tag);
    let done = false;
    const end = (loaded) => {
      if (done) return;
      done = true;
      element.remove();
      resolve(loaded);
    };
    element.addEventListener("load", () => end(true));
    element.addEventListener("error", () => end(false));
    for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, value);
    document.head.append(element);
    setTimeout(() => end(false), ms);
  });
}

async function probe(config) {
  const listener = `http://127.0.0.1:${config.port}`;
  const base = location.href.slice(0, location.href.indexOf("/test.hostile/"));

  // A1: the application's own ways in. Where the system puts the application's script
  // into frames too, its calls are made: ones the test can see the effect of (a new tab,
  // a setting written), with no token and with a made-up one.
  const internals = typeof window.__TAURI_INTERNALS__ !== "undefined";
  note("A1.internals-present", internals ? "present (put in by the system)" : "absent");
  if (internals) {
    const forged = { headers: { "lume-frame": "0".repeat(64), "lume-carrier": "1" } };
    for (const [what, command, args, options] of [
      ["list", "commands_list", {}, undefined],
      ["tab", "command_run", { carrier: 1, id: "lume.tab.new", args: {} }, undefined],
      ["tab forged", "command_run", { carrier: 1, id: "lume.tab.new", args: {} }, forged],
      ["setting", "config_set", { carrier: 1, path: "terminal.font-size", value: 31 }, forged],
      ["token", "frame_token", {}, undefined],
    ]) {
      const tried = await attempt(() => window.__TAURI_INTERNALS__.invoke(command, args, options));
      record(
        `A1.invoke ${what}`,
        !tried.threw,
        tried.threw ? tried.value : JSON.stringify(tried.value).slice(0, 100),
      );
    }
  }
  record("A1.global", typeof window.__TAURI__ !== "undefined", typeof window.__TAURI__);
  note("A1.ipc", typeof window.ipc);
  const webview = window.chrome && window.chrome.webview;
  note("A1.webview2", webview ? "chrome.webview is present" : "absent");
  if (webview) {
    // A forged request straight to the system's channel, as the application's own
    // script would send it; its effect, a new tab, is what the test looks for.
    const posted = await attempt(async () => {
      webview.postMessage(
        JSON.stringify({
          cmd: "command_run",
          callback: 1,
          error: 2,
          payload: { carrier: 1, id: "lume.tab.new", args: {} },
          options: { headers: { "lume-frame": "0".repeat(64) } },
        }),
      );
      return "posted";
    });
    note("A1.webview2-post", posted.value);
  }
  const webkit = window.webkit && window.webkit.messageHandlers;
  note("A1.webkit", webkit ? Object.keys(webkit).join(",") : "absent");

  // A2: the application's commands by address, with a forged body.
  for (const address of ["http://ipc.localhost/commands_list", "ipc://localhost/commands_list"]) {
    const tried = await attempt(() =>
      fetch(address, {
        method: "POST",
        body: "{}",
        headers: { "Content-Type": "application/json", "Tauri-Callback": "1", "Tauri-Error": "2" },
      }),
    );
    record(
      `A2.fetch ${address}`,
      !tried.threw,
      tried.value && tried.value.status ? `status ${tried.value.status}` : tried.value,
    );
  }
  // A beacon is queued whatever happens to it later, so whether it went is told by
  // the listener, and by the application's log for its own address.
  const beacon = await attempt(() =>
    navigator.sendBeacon("http://ipc.localhost/commands_list", "{}"),
  );
  note("A2.beacon-ipc", `queued: ${beacon.value}`);
  const beaconOut = await attempt(() => navigator.sendBeacon(`${listener}/beacon`, "x"));
  note("A2.beacon-out", `queued: ${beaconOut.value}`);

  // A3: the window around the frame.
  const doc = await attempt(() => parent.document.title);
  record("A3.parent.document", !doc.threw, doc.value);
  const top = await attempt(() => window.top.location.href);
  record("A3.top.location", !top.threw, top.value);
  record("A3.opener", window.opener !== null, String(window.opener));
  const frameElement = await attempt(() => window.frameElement);
  record("A3.frameElement", !frameElement.threw && frameElement.value !== null, frameElement.value);

  // A4: navigating the window, and popups.
  const opened = await attempt(() => window.open(`${listener}/popup`));
  record("A4.open", !opened.threw && opened.value !== null, opened.value);
  const topNav = await attempt(() => {
    window.top.location.href = `${listener}/top`;
    return "assigned";
  });
  record("A4.top-navigation", !topNav.threw, topNav.value);

  // A5: the page's token. Nothing of the window's is on this one's global object, and
  // asking for the token is refused like any other command.
  const hex = Object.getOwnPropertyNames(window).filter((name) => {
    const value = (() => {
      try {
        return window[name];
      } catch {
        return undefined;
      }
    })();
    return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
  });
  record("A5.globals", hex.length > 0, hex.join(",") || "none");
  const token = await attempt(() =>
    fetch("http://ipc.localhost/frame_token", { method: "POST", body: "{}" }),
  );
  record(
    "A5.ask",
    !token.threw,
    token.value && token.value.status ? `status ${token.value.status}` : token.value,
  );
  parent.postMessage({ lume: "connect", give: "token" }, "*");

  // A6: another plugin's name in what is sent.
  const who = await call("whoami", { plugin: "test.hello", instance: "w-1/v999" });
  record(
    "A6.identity",
    who.plugin !== config.self || who.from.instance === "w-1/v999",
    `answered as ${who.plugin} for ${who.from.instance}`,
  );

  // A7: other plugins' files and the window's own.
  for (const [what, address] of [
    ["another plugin", `${base}/test.hello/host.js`],
    ["by climbing", `${base}/test.hostile/../test.hello/host.js`],
    ["encoded", `${base}/test.hostile/%2e%2e/test.hello/host.js`],
    ["the window's page", "http://tauri.localhost/index.html"],
    ["the window's page (dev)", "http://localhost:1420/src/main.tsx"],
  ]) {
    const loaded = await loads("script", { src: address });
    record(`A7.script ${what}`, loaded, address);
  }
  const ownLoaded = await loads("script", { src: `${base}/test.hostile/ui/probe-ok.js` });
  record(
    "A7.own-files",
    !ownLoaded,
    ownLoaded ? "the plugin's own file loaded, as it should" : "the plugin's own file did not load",
  );

  // A8: the network, every way a page has.
  const fetched = await attempt(() => fetch(`${listener}/fetch`));
  record("A8.fetch", !fetched.threw, fetched.value);
  const socket = await attempt(() => connects(new WebSocket(`ws://127.0.0.1:${config.port}/ws`)));
  record("A8.websocket", !socket.threw && socket.value === true, socket.value);
  const source = await attempt(() => connects(new EventSource(`${listener}/events`)));
  record("A8.eventsource", !source.threw && source.value === true, source.value);
  const image = await loads("img", { src: `${listener}/image.png` });
  record("A8.image", image, "image beacon");
  const font = await attempt(async () => {
    const face = new FontFace("Beacon", `url(${listener}/font.woff2)`);
    await face.load();
    return "loaded";
  });
  record("A8.font", !font.threw, font.value);
  const prefetch = await loads("link", { rel: "prefetch", href: `${listener}/prefetch` }, 1000);
  record("A8.prefetch", prefetch, "prefetch");
  const form = await attempt(() => {
    const element = document.createElement("form");
    element.action = `${listener}/form`;
    element.method = "POST";
    document.body.append(element);
    element.submit();
    return "submitted";
  });
  record("A8.form", false, `form: ${form.value} (whether it left is told by the listener)`);
  // A frame of its own: refused by the policy; a refused frame still fires its load
  // event, so whether it reached anything is told by the listener.
  await loads("iframe", { src: `${listener}/frame` });
  note("A8.frame", "a frame of its own was asked for");
  const evaluated = await attempt(() => eval("1 + 1"));
  record("A8.eval", !evaluated.threw, evaluated.value);
  const fn = await attempt(() => new Function("return 2")());
  record("A8.function", !fn.threw, fn.value);
  record("A8.inline-script", window.inlineRan === true, String(window.inlineRan));
  const worker = await attempt(() => new Worker(`${listener}/worker.js`));
  record("A8.worker-elsewhere", !worker.threw, worker.value);

  // A9: storage.
  const cookie = await attempt(() => {
    document.cookie = "a=1";
    return document.cookie;
  });
  record("A9.cookie", !cookie.threw && String(cookie.value).includes("a=1"), cookie.value);
  const local = await attempt(() => {
    window.localStorage.setItem("a", "1");
    return window.localStorage.getItem("a");
  });
  record("A9.localStorage", !local.threw && local.value === "1", local.value);
  const session = await attempt(() => {
    window.sessionStorage.setItem("a", "1");
    return window.sessionStorage.getItem("a");
  });
  record("A9.sessionStorage", !session.threw && session.value === "1", session.value);
  const indexed = await attempt(
    () =>
      new Promise((resolve, reject) => {
        const request = indexedDB.open("probe");
        request.onsuccess = () => resolve("opened");
        request.onerror = () => reject(request.error);
      }),
  );
  record("A9.indexedDB", !indexed.threw, indexed.value);
  const service = await attempt(() =>
    navigator.serviceWorker
      ? navigator.serviceWorker.register("probe.js")
      : Promise.reject(new Error("no service workers")),
  );
  record("A9.serviceWorker", !service.threw, service.value);
  const cache = await attempt(() =>
    window.caches ? caches.open("probe") : Promise.reject(new Error("no caches")),
  );
  record("A9.caches", !cache.threw, cache.value);

  // A10: what the system would ask the user for.
  const clip = await attempt(() => navigator.clipboard.readText());
  record("A10.clipboard-read", !clip.threw, clip.value);
  const clipWrite = await attempt(() => navigator.clipboard.writeText("probe"));
  record("A10.clipboard-write", !clipWrite.threw, clipWrite.value);
  const where = await attempt(
    () =>
      new Promise((resolve, reject) => {
        if (!navigator.geolocation) return reject(new Error("no geolocation"));
        navigator.geolocation.getCurrentPosition(resolve, reject, { timeout: 1000 });
        return undefined;
      }),
  );
  record("A10.geolocation", !where.threw, where.value);
  const media = await attempt(() => navigator.mediaDevices.getUserMedia({ video: true }));
  record("A10.camera", !media.threw, media.value);
  const notify = await attempt(() =>
    typeof Notification === "undefined" ? "denied" : Notification.requestPermission(),
  );
  record("A10.notifications", notify.value === "granted", notify.value);

  // A12: a document inside this one, trying again.
  const nestedScript =
    "<script>parent.postMessage('nested ran', '*'); fetch('" +
    listener +
    "/nested');</" +
    "script>";
  const srcdoc = await nestedRuns((frame) => {
    frame.srcdoc = nestedScript;
  });
  record("A12.srcdoc", srcdoc, srcdoc ? "a script in a nested document ran" : "nothing ran in it");
  const blob = await nestedRuns((frame) => {
    frame.src = URL.createObjectURL(new Blob([nestedScript], { type: "text/html" }));
  });
  record("A12.blob-frame", blob, blob ? "a script in a nested document ran" : "nothing ran in it");
  const data = await nestedRuns((frame) => {
    frame.src = "data:text/html," + encodeURIComponent(nestedScript);
  });
  record("A12.data-frame", data, data ? "a script in a nested document ran" : "nothing ran in it");

  // A13: keys this page says were pressed and that it had no use for. Each is bound
  // to something a view must not be able to set off: pasting, a new tab, closing the
  // pane, a new window, quitting. None was pressed; what they came to is for whoever
  // reads the application afterwards to say, so they are only noted here.
  const forged = [
    ["V", "KeyV"],
    ["T", "KeyT"],
    ["W", "KeyW"],
    ["N", "KeyN"],
    ["E", "KeyE"],
    ["O", "KeyO"],
  ];
  for (const [key, code] of forged) {
    for (const mods of [["ctrl", "shift"], ["ctrl"], ["alt"], ["meta"], ["meta", "shift"]]) {
      port.postMessage({ t: "key", key, code, mods });
    }
  }
  // Whatever they set off has happened by the time the report is read.
  await settle(500);
  note("A13.forged-keys", `${forged.length * 5} keys that were never pressed were passed on`);

  // A14: a stream another plugin's view was let use. The grant is a real one, which
  // the test took from that plugin's log; it was made for that view and no other.
  if (config.grant) {
    const stream = await attempt(() => ask({ t: "stream", grant: config.grant }));
    record(
      "A14.foreign-grant",
      !stream.threw,
      stream.threw ? stream.value : "another plugin's stream was opened",
    );
  } else {
    note("A14.foreign-grant", "no grant was given to try");
  }

  await call("report", { lines: results, done: true });
  document.getElementById("state").textContent = "done";
  await settle(200);

  // Last, as it takes the page away: its own frame sent elsewhere.
  location.href = `${listener}/navigate`;
}

window.addEventListener("message", (event) => {
  if (port !== null || !event.data || event.data.lume !== "connect" || event.ports.length !== 1)
    return;
  port = event.ports[0];
  port.onmessage = (message) => {
    const data = message.data;
    const found = waiting.get(data.id);
    if (!found) return;
    waiting.delete(data.id);
    if (data.t === "result") found.resolve(data.value);
    else if (data.t === "opened") found.resolve("opened");
    else found.reject(new Error(`${data.code}: ${data.message}`));
  };
  port.postMessage({ t: "ready" });
  call("config", null)
    .then(probe)
    .catch((error) => {
      port.postMessage({
        t: "call",
        id: 0,
        name: "report",
        args: { lines: [{ id: "probe", outcome: "failed", detail: String(error) }], done: true },
      });
    });
});
