// The echo view's code: it opens the view, hands the view streams to use, and writes
// what the view reports to its log, one line each.
const said = (line) => lume.log.info(line);

// The file the view reads as a stream, in the plugin's own folder.
const FLOW = "${plugin-data}/flow.bin";
// 64 KiB of text.
const PIECE = "0123456789abcdef".repeat(4096);

lume.commands.register("open", async (args) => {
  const opened = await lume.ui.openView("echo", {
    state: args?.state ?? null,
    place: args?.place,
    title: args?.title,
    reuse: args?.reuse === true,
  });
  said(`opened pane ${opened.pane} reused ${opened.reused}`);
});

// A program whose input and output the view uses itself.
lume.views.handle("sort", async (_args, from) => {
  const child = await lume.process.spawn("sort");
  return {
    input: await lume.streams.grantToView(from.instance, child.stdin),
    output: await lume.streams.grantToView(from.instance, child.stdout),
  };
});

// A grant made for another of the plugin's open views than the one that asks.
lume.views.handle("grant-for", async (args) => {
  const child = await lume.process.spawn("sort");
  return { grant: await lume.streams.grantToView(args.instance, child.stdout) };
});

lume.views.handle("instances", () => lume.views.instances());

// A failure of a kind that is the plugin's own, with something said of it.
lume.views.handle("fail", () => {
  throw new lume.LumeError("echo.own-kind", "a failure of its own kind", { count: 7 });
});

// A grant the view never opens, written to the log for another plugin's view to try.
lume.views.handle("spare", async (_args, from) => {
  const child = await lume.process.spawn("sort");
  const grant = await lume.streams.grantToView(from.instance, child.stdout);
  said(`spare grant ${grant}`);
  return null;
});

lume.views.handle("revoke", async (args) => {
  await lume.streams.revoke(args.grant);
  return null;
});

// A file of so many bytes, as a stream the view reads.
lume.views.handle("file", async (args, from) => {
  if (args.write !== false) {
    await lume.fs.write(FLOW, "");
    for (let written = 0; written < args.bytes; written += PIECE.length)
      await lume.fs.write(FLOW, PIECE, { mode: "append" });
  }
  const stream = await lume.fs.openStream(FLOW);
  return { grant: await lume.streams.grantToView(from.instance, stream) };
});

// The same file by way of this code: read here, and posted to the view piece by piece.
lume.views.handle("relay", (_args, from) => {
  (async () => {
    const stream = await lume.fs.openStream(FLOW);
    for (;;) {
      const piece = await lume.streams.read(stream, { max: 65536, encoding: "base64" });
      if (piece === null) break;
      lume.views.post(from.instance, "piece", piece);
    }
    await lume.streams.close(stream);
    lume.views.post(from.instance, "end", null);
  })().catch((error) => said(`relay failed: ${error.message}`));
  return null;
});

lume.views.handle("report", (args) => {
  for (const line of args.lines ?? []) said(`echo ${line}`);
  return null;
});

// How long the window's page takes to answer, so many times over. The command asked
// for is one the page carries out and that changes nothing here (the text is at its
// usual size already), so each answer is one the page gave between its other work.
lume.commands.register("page-answers", async (args) => {
  const samples = [];
  for (let n = 0; n < (args?.count ?? 100); n++) {
    const before = Date.now();
    await lume.commands.execute("lume.view.zoom-reset");
    samples.push(Date.now() - before);
  }
  samples.sort((a, b) => a - b);
  const at = (share) => samples[Math.round((samples.length - 1) * share)];
  const figures = { count: samples.length, p50: at(0.5), p95: at(0.95), max: at(1) };
  said(`page-answers ${args?.label} ${JSON.stringify(figures)}`);
});

lume.views.onOpen((view) => said(`view opened ${view.view} in a ${view.place}`));
lume.views.onClose((view) => said(`view closed ${view.view}`));
