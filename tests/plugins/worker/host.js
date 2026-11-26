// Uses each of the things it was given once, and logs what came of them on one line.
const soon = (ms) => new Promise((done) => lume.timers.setTimeout(done, ms));

lume.commands.register("work", async (args) => {
  const said = [];

  // Another plugin's service: that plugin is started to answer.
  const answer = await lume.services.call("test.provider", "double", { n: 21 });
  said.push(`service ${answer.twice} from ${answer.from}`);

  // Its own folder, watched while a file in it is written.
  await lume.fs.write("${plugin-data}/seen.txt", "0");
  let events = 0;
  const watch = await lume.fs.watch("${plugin-data}", () => {
    events++;
  });
  await lume.fs.write("${plugin-data}/seen.txt", "1");
  for (let waited = 0; waited < 100 && events === 0; waited++) await soon(50);
  watch.dispose();
  said.push(`watched ${events > 0}`);

  // A program left running, written to and read from through its streams.
  const child = await lume.process.spawn("sort");
  // An odd number of bytes: Windows' sort takes input it can read as UTF-16 for UTF-16.
  await lume.streams.write(child.stdin, "beta\nalpha\n");
  await lume.streams.close(child.stdin);
  let sorted = "";
  for (;;) {
    const piece = await lume.streams.read(child.stdout);
    if (piece === null) break;
    sorted += piece;
  }
  const exit = await lume.process.wait(child.process);
  said.push(`sorted ${sorted.trim().split(/\r?\n/).join(",")} exit ${exit.code}`);

  // What the programs of a pane use of the machine.
  const stats = await lume.panes.stats(args.pane);
  said.push(`processes ${stats.processes > 0} memory ${stats.memory > 0}`);

  lume.log.info(said.join("; "));
});
