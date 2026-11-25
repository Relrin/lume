// Says where the sessions are, asks to run programs there, and runs some. What came of
// each is logged under the mark it was asked for with.
const why = (error) => error.code;
const say = (args, what) => lume.log.info(`${args.mark} ${JSON.stringify(what)}`);

lume.commands.register("look", async (args) => {
  const sessions = await lume.sessions.list();
  const places = sessions.map((session) => session.place).filter((place) => place !== undefined);
  say(args, { places, machines: await lume.machines.list() });
});

lume.commands.register("allow", async (args) => {
  say(args, { allowed: await lume.machines.allow(args.machine, "to run uname").catch(why) });
});

lume.commands.register("work", async (args) => {
  const { machine } = args;
  const run = (program, words, options) =>
    lume.process
      .run(program, words, { machine, ...options })
      .then(
        (result) => `${result.code} ${result.stdout.trim()} ${result.stderr.trim()}`.trim(),
        why,
      );
  const said = {
    uname: await run("uname", ["-s"]),
    input: await run("cat", [], { stdin: "åäö 'quoted' $HOME" }),
    folder: await run("sh", ["-c", "pwd; echo err >&2; exit 3"], { cwd: "/tmp" }),
    nowhere: await run("uname", [], { cwd: "/no such folder" }),
    local: await run("uname", [], { cwd: "${home}" }),
    other: await run("git", ["--version"]),
    here: await lume.process.run("uname").then(() => "ran", why),
  };
  if (!said.uname.startsWith("0 ")) return say(args, said);

  // One that is left running, read as it writes.
  const words = ["-c", "for n in 1 2 3; do echo $n; sleep 0.2; done; exit 4"];
  const child = await lume.process.spawn("sh", words, { machine });
  said.pid = child.pid;
  said.write = await lume.streams.write(child.stdin, "x").then(() => "written", why);
  said.pieces = "";
  for (;;) {
    const piece = await lume.streams.read(child.stdout);
    if (piece === null) break;
    said.pieces += piece;
  }
  said.exit = (await lume.process.wait(child.process)).code;
  say(args, said);
});
