// The hostile view's code: it tells its page where the test listens, says who it is,
// and writes each of the page's reports to its log.
lume.views.handle("config", async () => {
  const settings = await lume.settings.get();
  return {
    port: settings.port,
    grant: settings.grant,
    abuse: settings.abuse,
    self: lume.self.id,
  };
});

lume.views.handle("abusing", (args) => {
  lume.log.info(`abuse ${args.mode} started`);
  return null;
});

// Takes whatever it is sent and says nothing of it.
lume.views.handle("sink", () => null);

lume.views.handle("whoami", (args, from) => ({ plugin: lume.self.id, from, asked: args }));

lume.views.handle("report", (args) => {
  for (const line of args.lines ?? [])
    lume.log.info(`probe ${line.id} ${line.outcome}: ${line.detail}`);
  if (args.done) lume.log.info("probe done");
  return null;
});

lume.views.onOpen((view) => lume.log.info(`view opened ${view.view} in a ${view.place}`));
lume.views.onClose((view) => lume.log.info(`view closed ${view.view}`));
