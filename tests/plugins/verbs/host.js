// Verbs for the command line. Each command is called with { argv, cwd, stdin } first
// and gives back what to print; one that gives `next` is called again with it.
const soon = (ms) => new Promise((done) => lume.timers.setTimeout(done, ms));

// Two steps: the arguments, then what was piped in. What the second step needs
// travels in `next`.
lume.commands.register("echo", async (args) => {
  if (args.next) {
    return {
      out: `stdin ${JSON.stringify(args.next.stdin)}\n`,
      err: "echoed\n",
    };
  }
  return {
    out: `argv ${JSON.stringify(args.argv)}\n`,
    err: `cwd ${args.cwd.length > 0}\n`,
    next: { stdin: args.stdin ?? null },
  };
});

lume.commands.register("table", async () => {
  const profiles = await lume.settings.get("profiles");
  return { out: `${JSON.stringify(profiles)}\n` };
});

lume.commands.register("fail", async () => ({ err: "it failed\n", code: 7 }));

// Never ends by itself: each call waits a little and asks to be called again, until
// one says the run was cancelled.
lume.commands.register("hang", async (args) => {
  if (!args.next) {
    lume.log.info("hang started");
    return { out: "started\n", next: { run: 1, calls: 0 } };
  }
  if (args.cancel) {
    lume.log.info(`hang cancelled after ${args.next.calls} calls`);
    return {};
  }
  await soon(200);
  return { next: { run: args.next.run, calls: args.next.calls + 1 } };
});

lume.commands.register("broken", async () => "not a step");

lume.commands.register("throw", async (args) => {
  if (args.next) throw new Error("the second step went wrong");
  return { out: "first\n", next: 1 };
});

lume.settings.onChange((settings) => {
  lume.log.info(`profiles now ${JSON.stringify(settings.profiles)}`);
});
