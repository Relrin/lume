// Counts how often it is greeted: kept with lume.kv, shown in a status item.
lume.commands.register("greet", async () => {
  const count = ((await lume.kv.get("count")) ?? 0) + 1;
  await lume.kv.set("count", count);
  await lume.status.set("count", { text: String(count) });
  lume.log.info("greeted", count);
  return count;
});
