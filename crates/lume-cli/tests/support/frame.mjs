// Evaluates an expression inside one page of a webview that was started with a
// debugging port, and prints what it gave as JSON. The end-to-end tests use it to look
// into a plugin's view, which is a page of its own that nothing else can read.
//
//   node frame.mjs <port> <part of the page's address> <expression>
//
// Exits with 2 when no page with such an address is there (yet), and with 1 when the
// expression failed or the port did not answer.

const [port, part, expression] = process.argv.slice(2);

function fail(code, message) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

if (!port || !part || !expression) fail(1, "usage: frame.mjs <port> <address part> <expression>");

let targets;
try {
  targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
} catch (error) {
  fail(1, `the debugging port ${port} did not answer: ${error.message}`);
}
const target = targets.find((one) => typeof one.url === "string" && one.url.includes(part));
if (!target) fail(2, `no page whose address holds ${part}`);

const socket = new WebSocket(target.webSocketDebuggerUrl);
const timer = setTimeout(() => fail(1, "the page did not answer in 10 seconds"), 10_000);
socket.addEventListener("error", () => fail(1, "the page's debugging socket failed"));
socket.addEventListener("open", () => {
  socket.send(
    JSON.stringify({
      id: 1,
      method: "Runtime.evaluate",
      params: { expression, returnByValue: true, awaitPromise: true },
    }),
  );
});
socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.id !== 1) return;
  clearTimeout(timer);
  socket.close();
  if (message.error) fail(1, message.error.message);
  const { result, exceptionDetails } = message.result;
  if (exceptionDetails) {
    fail(1, exceptionDetails.exception?.description ?? exceptionDetails.text);
  }
  process.stdout.write(`${JSON.stringify(result.value ?? null)}\n`);
});
