// Opening a terminal starts a session, which the layout permission alone does not
// allow: with a shell, with a program, as a pane, a tab or a window. Each try must be
// refused as denied; the command fails naming any that was not.
lume.commands.register("probe", async () => {
  const panes = await lume.panes.list();
  const pane = panes[0].pane;
  const shell = { type: "terminal" };
  const program = {
    type: "terminal",
    launcher: { type: "shell", program: "git", args: ["--version"], login: false },
  };
  const tries = {
    "a split with a shell": { type: "split", pane, dir: "right", ratio: 500000, content: shell },
    "a split with a program": {
      type: "split",
      pane,
      dir: "right",
      ratio: 500000,
      content: program,
    },
    "a window with a program": { type: "window_new", content: program },
  };
  const wrong = [];
  for (const [name, intent] of Object.entries(tries)) {
    try {
      await lume.layout.apply(intent);
      wrong.push(`${name} was allowed`);
    } catch (error) {
      if (error.code !== "denied") wrong.push(`${name} failed as ${error.code}: ${error.message}`);
    }
  }
  // What the permission does cover still goes through.
  await lume.layout.apply({ type: "focus", target: { type: "pane", pane } });
  if (wrong.length > 0) throw new Error(wrong.join("; "));
  lume.log.info(`refused all ${Object.keys(tries).length}`);
});
