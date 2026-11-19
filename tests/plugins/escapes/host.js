// Tries to reach past what it was given. Each escape must be refused: as denied, or, for
// a path that climbs with `..`, as not a path at all. What it was given must work. The
// command fails naming anything that went otherwise.
const escapes = {
  "a path that climbs out of its data folder": [
    () => lume.fs.read("${plugin-data}/../../lock.toml"),
    ["denied", "invalid"],
  ],
  "a link in its data folder to Lume's own folder": [
    () => lume.fs.read("${plugin-data}/link/lock.toml"),
    ["denied"],
  ],
  "a workspace with no root": [() => lume.fs.list("${workspace}"), ["denied"]],
  "a server it was not given": [() => lume.net.fetch("http://127.0.0.1:9/"), ["denied"]],
  "this computer by name": [() => lume.net.fetch("http://localhost:9/"), ["denied"]],
  "a program it was not given": [() => lume.process.run("cmd", ["/c", "echo"]), ["denied"]],
  "Lume itself": [() => lume.process.run("lume", ["--version"]), ["denied"]],
  "writing outside its data folder": [
    () => lume.fs.write("${home}/lume-escape.txt", "x"),
    ["denied"],
  ],
};

lume.commands.register("probe", async () => {
  const wrong = [];
  for (const [name, [attempt, refusals]] of Object.entries(escapes)) {
    try {
      await attempt();
      wrong.push(`${name} was allowed`);
    } catch (error) {
      if (!refusals.includes(error.code)) {
        wrong.push(`${name} failed as ${error.code}: ${error.message}`);
      }
    }
  }
  // What it was given works.
  await lume.fs.write("${plugin-data}/kept.txt", "kept");
  const read = await lume.fs.read("${plugin-data}/kept.txt");
  if (read !== "kept") wrong.push(`its own file read back as ${JSON.stringify(read)}`);
  if (wrong.length > 0) throw new Error(wrong.join("; "));
  lume.log.info(`refused all ${Object.keys(escapes).length}`);
});
