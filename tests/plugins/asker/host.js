// Asks the user for a permission while it runs, and types into a pane, which the user
// approves each time.
lume.commands.register("request", async () => {
  const granted = await lume.permissions.request(
    "workspace.read",
    undefined,
    "to say which workspace this is",
  );
  lume.log.info(`granted ${granted}`);
  if (granted) {
    const workspace = await lume.workspace.current();
    lume.log.info(`workspace ${workspace.name}`);
  }
  return granted;
});

lume.commands.register("type", async (args) => {
  await lume.panes.write(args.pane, args.text);
  lume.log.info("typed");
});
