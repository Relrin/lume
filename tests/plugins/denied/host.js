// Tries every call that needs a permission, holding none, and says how each went. Each
// must be refused as denied; the command fails naming any that was not.
const tries = {
  "fs.read": () => lume.fs.read("${home}/.profile"),
  "fs.write": () => lume.fs.write("${plugin-data}/x.txt", "x"),
  "fs.stat": () => lume.fs.stat("${home}"),
  "fs.list": () => lume.fs.list("${home}"),
  "fs.watch": () => lume.fs.watch("${home}", () => {}),
  "fs.openStream": () => lume.fs.openStream("${home}/.profile"),
  "process.run": () => lume.process.run("git", ["--version"]),
  "process.spawn": () => lume.process.spawn("git", ["--version"]),
  "net.fetch": () => lume.net.fetch("https://example.org/"),
  "net.fetchStream": () => lume.net.fetchStream("https://example.org/"),
  "clipboard.read": () => lume.clipboard.read(),
  "clipboard.write": () => lume.clipboard.write("x"),
  "panes.list": () => lume.panes.list(),
  "panes.text": () => lume.panes.text("p1"),
  "panes.write": () => lume.panes.write("p1", "x"),
  "panes.stats": () => lume.panes.stats("p1"),
  "sessions.spawn": () => lume.sessions.spawn(),
  "sessions.list": () => lume.sessions.list(),
  "workspace.current": () => lume.workspace.current(),
  "workspace.root": () => lume.workspace.root(),
  "layout.apply": () => lume.layout.apply({ type: "focus", target: { type: "pane", pane: "p1" } }),
  "metrics.snapshot": () => lume.metrics.snapshot(),
  "services.call": () => lume.services.call("test.other", "anything"),
  "permissions.request": () => lume.permissions.request("clipboard.read"),
};

lume.commands.register("probe", async () => {
  const wrong = [];
  for (const [name, attempt] of Object.entries(tries)) {
    try {
      await attempt();
      wrong.push(`${name} was allowed`);
    } catch (error) {
      if (error.code !== "denied") wrong.push(`${name} failed as ${error.code}: ${error.message}`);
    }
  }
  if (wrong.length > 0) throw new Error(wrong.join("; "));
  lume.log.info(`refused all ${Object.keys(tries).length}`);
});
