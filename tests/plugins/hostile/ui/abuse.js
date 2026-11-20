// The hostile view's second page: it does not try to get out, it tries to get in the
// way. As its plugin's settings say, it floods the channel with small messages, sends
// large ones as fast as a view may, or never gives up the processor.

"use strict";

let port = null;

function flood() {
  // As many as will go, for as long as the channel is there.
  let n = 0;
  const go = () => {
    for (let i = 0; i < 20000; i++) port.postMessage({ t: "state", state: n++ });
    setTimeout(go, 0);
  };
  go();
}

function large() {
  // Each as large as a message may be, at the rate messages may be sent.
  const filler = "x".repeat(900 * 1024);
  let id = 1000;
  setInterval(() => {
    for (let i = 0; i < 2; i++)
      port.postMessage({ t: "call", id: id++, name: "sink", args: filler });
  }, 10);
}

function spin() {
  for (;;) {
    // Never yields.
  }
}

window.addEventListener("message", (event) => {
  if (port !== null || !event.data || event.data.lume !== "connect" || event.ports.length !== 1)
    return;
  port = event.ports[0];
  port.onmessage = (message) => {
    const data = message.data;
    if (data.t !== "result" || data.id !== 1) return;
    const mode = data.value.abuse;
    port.postMessage({ t: "call", id: 2, name: "abusing", args: { mode } });
    // The report is on its way before the abuse begins.
    setTimeout(() => {
      if (mode === "large") large();
      else if (mode === "spin") spin();
      else flood();
    }, 200);
  };
  port.postMessage({ t: "ready" });
  port.postMessage({ t: "call", id: 1, name: "config", args: null });
});
