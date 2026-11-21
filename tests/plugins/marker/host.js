// A pane is given a picture by its name: one of the plugin's own, or one of the
// application's tiles. A picture of another plugin's is refused.
const first = async () => (await lume.panes.list())[0].pane;

const give = async (icon) => lume.layout.apply({ type: "pane_icon", pane: await first(), icon });

lume.commands.register("own", async () => {
  await give("mark");
  try {
    await give("image:test.other:mark");
  } catch (error) {
    if (error.code !== "invalid") throw error;
    lume.log.info("gave its own and was refused another's");
    return;
  }
  throw new Error("another plugin's picture was allowed");
});

lume.commands.register("tile", async () => {
  await give("agent");
  lume.log.info("gave a tile");
});

lume.commands.register("clear", async () => {
  await give(null);
  lume.log.info("took it away");
});
