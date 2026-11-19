// A plugin written against the typings, as an author would write one. The typecheck
// fails when the typings no longer describe what a plugin may write.

import type { SessionReport, StatusValue } from "../src/index.d.ts";

lume.commands.register("refresh", async (args) => {
  const count = ((await lume.kv.get("count")) as number | null) ?? 0;
  await lume.kv.set("count", count + 1);
  const value: StatusValue = {
    text: String(count),
    value: 0.5,
    tone: "info",
    card: [{ label: "Count", value: String(count) }],
  };
  await lume.status.set("cpu", value);
  await lume.status.clear("cpu");
  await lume.commands.execute("lume.palette.open");
  await lume.context.set("plugin:acme.meters.busy", true);
  const settings = await lume.settings.get();
  const keys: string[] = await lume.kv.list();
  lume.log.info("refreshed", args, settings, keys, lume.self.id);
  return count;
});

const every = lume.timers.setInterval(() => console.log("tick"), 1000);
lume.timers.clearInterval(every);
const listening = lume.sessions.onEvent((report: SessionReport) => {
  if (report.kind === "command_finished") lume.log.warn(report.session, report.pane);
});
listening.dispose();
lume.settings.onChange((all) => lume.log.debug(all));
lume.onDeactivate(() => lume.log.info("bye"));

async function wrong(): Promise<void> {
  // @ts-expect-error a status value has no colour
  await lume.status.set("cpu", { colour: "red" });
  // @ts-expect-error a key is a string
  await lume.kv.get(42);
  try {
    await lume.kv.delete("x");
  } catch (error) {
    if (error instanceof lume.LumeError && error.code === "denied") lume.log.error(error.message);
  }
}
void wrong;
