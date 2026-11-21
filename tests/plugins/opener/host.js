// A terminal that runs a program is opened for the program the plugin may run, beside
// the pane it names, and refused for any other program, by either way of asking.
lume.commands.register("probe", async () => {
  const before = await lume.panes.list();
  const pane = before[0].pane;
  const wrong = [];

  const opened = await lume.sessions.spawn(undefined, {
    program: "git",
    args: ["--version"],
    env: { LUME_TEST: "1" },
    place: "right",
    pane,
  });
  if (typeof opened.pane !== "string" || opened.pane === pane) wrong.push("no new pane was given");

  const refused = async (name, attempt, code) => {
    try {
      await attempt();
      wrong.push(`${name} was allowed`);
    } catch (error) {
      if (error.code !== code) wrong.push(`${name} failed as ${error.code}: ${error.message}`);
    }
  };
  await refused(
    "another program",
    () => lume.sessions.spawn(undefined, { program: "ssh", place: "right", pane }),
    "denied",
  );
  await refused(
    "another program through the layout",
    () =>
      lume.layout.apply({
        type: "split",
        pane,
        dir: "down",
        ratio: 500000,
        content: {
          type: "terminal",
          launcher: { type: "shell", program: "ssh", args: [], login: false },
        },
      }),
    "denied",
  );
  await refused(
    "another machine through the layout",
    () =>
      lume.layout.apply({
        type: "split",
        pane,
        dir: "down",
        ratio: 500000,
        content: { type: "terminal", launcher: { type: "ssh", host: "example.invalid" } },
      }),
    "denied",
  );
  await refused(
    "a path in place of a name",
    () => lume.sessions.spawn(undefined, { program: "../git", place: "right", pane }),
    "denied",
  );
  await refused(
    "arguments with no program",
    () => lume.sessions.spawn(undefined, { args: ["--version"] }),
    "invalid",
  );

  if (wrong.length > 0) throw new Error(wrong.join("; "));
  lume.log.info("opened one and refused the rest");
});
