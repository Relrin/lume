// Presses keys in one page of a webview that was started with a debugging port, as a
// keyboard does: the page's own handlers and the browser's are given them alike.
//
//   node keys.mjs <port> <part of the page's address> <key>...
//
// A key is a name, such as `Enter` or `ArrowDown`, with modifiers before it, such as
// `Ctrl+Shift+C`; `text:` and something is typed as it is. Exits with 2 when no page
// with such an address is there (yet), and with 1 when the port did not answer.

const [port, part, ...keys] = process.argv.slice(2);

function fail(code, message) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

if (!port || !part || keys.length === 0) fail(1, "usage: keys.mjs <port> <address part> <key>...");

let targets;
try {
  targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
} catch (error) {
  fail(1, `the debugging port ${port} did not answer: ${error.message}`);
}
const target = targets.find((one) => typeof one.url === "string" && one.url.includes(part));
if (!target) fail(2, `no page whose address holds ${part}`);

const MODIFIERS = { Alt: 1, Ctrl: 2, Meta: 4, Shift: 8 };
const CODES = {
  Backspace: 8,
  Tab: 9,
  Enter: 13,
  Escape: 27,
  End: 35,
  Home: 36,
  ArrowLeft: 37,
  ArrowUp: 38,
  ArrowRight: 39,
  ArrowDown: 40,
  Delete: 46,
  F2: 113,
};

const socket = new WebSocket(target.webSocketDebuggerUrl);
setTimeout(() => fail(1, "the page did not answer in 10 seconds"), 10_000).unref();
socket.addEventListener("error", () => fail(1, "the page's debugging socket failed"));
await new Promise((opened) => socket.addEventListener("open", opened));

let last = 0;
const waiting = new Map();
socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.error) fail(1, message.error.message);
  waiting.get(message.id)?.();
});
function send(method, params) {
  last += 1;
  const id = last;
  return new Promise((done) => {
    waiting.set(id, done);
    socket.send(JSON.stringify({ id, method, params }));
  });
}

for (const pressed of keys) {
  if (pressed.startsWith("text:")) {
    await send("Input.insertText", { text: pressed.slice("text:".length) });
    continue;
  }
  const parts = pressed.split("+");
  const key = parts.pop();
  const modifiers = parts.reduce((sum, name) => sum + (MODIFIERS[name] ?? 0), 0);
  const code = CODES[key] ?? key.toUpperCase().charCodeAt(0);
  const event = {
    key,
    code: key.length === 1 ? `Key${key.toUpperCase()}` : key,
    windowsVirtualKeyCode: code,
    modifiers,
  };
  // Enter is what presses a button only if it carries what it types.
  const text = key === "Enter" && modifiers === 0 ? "\r" : undefined;
  await send("Input.dispatchKeyEvent", {
    type: text === undefined ? "rawKeyDown" : "keyDown",
    ...event,
    ...(text === undefined ? {} : { text }),
  });
  await send("Input.dispatchKeyEvent", { type: "keyUp", ...event });
}
socket.close();
process.exit(0);
