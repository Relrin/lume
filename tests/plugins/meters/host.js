// Shows the machine's readings in four status items. Which are on screen its settings
// say, and the window follows them itself; the machine is read only while one is.
const ITEMS = ["cpu", "memory", "gpu", "network"];
const HISTORY = 24;
const received = [];
let subscription = null;

function bytes(count) {
  const units = ["B", "KB", "MB", "GB"];
  let value = count;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function show(sample) {
  if (sample.cpu !== undefined) {
    lume.status.set("cpu", {
      value: sample.cpu,
      text: `${Math.round(sample.cpu * 100)}%`,
      color: "color.accent",
    });
  }
  if (sample.memory !== undefined) {
    lume.status.set("memory", {
      value: sample.memory.used / sample.memory.total,
      text: `${bytes(sample.memory.used)} / ${bytes(sample.memory.total)}`,
      color: "color.status.info",
    });
  }
  if (sample.gpu !== undefined) {
    const busy = sample.gpu.state === "read" ? sample.gpu.busy : undefined;
    lume.status.set("gpu", {
      value: busy ?? 0,
      text: busy === undefined ? "n/a" : `${Math.round(busy * 100)}%`,
      color: "color.tab.purple",
    });
  }
  if (sample.network !== undefined) {
    received.push(sample.network.received + sample.network.sent);
    while (received.length > HISTORY) received.shift();
    lume.status.set("network", {
      points: [...received],
      text: `↓ ${bytes(sample.network.received)}/s`,
      color: "color.tab.teal",
    });
  }
}

function apply(settings) {
  const shown = ITEMS.filter((item) => settings[item] !== false);
  subscription?.dispose();
  subscription = null;
  if (shown.length > 0) {
    subscription = lume.metrics.subscribe(shown, show, { while: shown });
  }
}

lume.commands.register("refresh", async () => {
  const sample = await lume.metrics.snapshot();
  if (sample) show(sample);
  return sample !== null;
});

lume.settings.onChange(apply);

lume.settings
  .get()
  .then(apply)
  .catch((error) => lume.log.error(error));
