// The hostile view's code: it tells its page where the test listens, says who it is,
// and writes each of the page's reports to its log.
lume.views.handle("config", async () => {
  const settings = await lume.settings.get();
  return { port: settings.port, self: lume.self.id };
});

lume.views.handle("whoami", (args, from) => ({ plugin: lume.self.id, from, asked: args }));

lume.views.handle("report", (args) => {
  for (const line of args.lines ?? [])
    lume.log.info(`probe ${line.id} ${line.outcome}: ${line.detail}`);
  if (args.done) lume.log.info("probe done");
  return null;
});

lume.views.onOpen((view) => lume.log.info(`view opened ${view.view} in a ${view.place}`));
