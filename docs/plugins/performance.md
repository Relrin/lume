# Performance

A terminal must start fast and stay out of the way. Lume is built so that an installed plugin costs
nothing until it is wanted, and this page says how to keep it that way.

The short version:

- Let the manifest do the showing. Run code only when something happens.
- Never use `onStartup` unless the plugin truly has work at every start.
- Hold no timer while there is nothing to do.
- Show everything that belongs together in one turn.
- Give a large tree its rows as they are opened.
- Read the machine only while someone is looking.

## What costs nothing

**An installed plugin does not slow the start.** Lume reads the installed plugins only after its
first terminal has shown its first output (or two seconds after the start, if no terminal shows
any). However many plugins are installed, the first prompt does not wait for them.

**What a plugin adds needs no code.** Menus, status items with their labels, panels, settings,
keys, themes, pictures, motions and cursor effects are all built from manifests. A plugin whose
code has never run still has its menu items in place and its settings on their page.

**Pictures and shaders are fetched when they are first needed.** A window is told only that a
picture or a shader exists. It fetches a picture the first time it draws it, and a shader when the
user chooses the effect. A plugin with fifty icons costs a window nothing for the ones it never
shows.

**Panel rows go only to windows that show the panel.** A window that does not have a panel on
screen is not sent its rows. When it brings the panel on screen, it is given them then.

## Choose activation events

A plugin's code starts when one of its activation events happens. Choose the narrowest that work.

| Event                   | Use it when                                                                |
| ----------------------- | -------------------------------------------------------------------------- |
| `onCommand:<name>`      | The code only does things the user asks for.                               |
| `onView:<name>`         | The code feeds a status item or a panel. It starts when that is on screen. |
| `onSessionEvent:<kind>` | The code reacts to sessions: a command finished, a folder changed.         |
| `onService:<name>`      | The code answers other plugins.                                            |
| `onStartup`             | The code must run at every start, whatever the user does.                  |

A command, a service call or the opening of a panel's row starts the code whether or not the
manifest lists it, so nothing needs `onStartup` to be reachable.

`onStartup` costs twice. The code starts at every launch, and it is never stopped for being idle.
Before reaching for it, ask what the code would do at start:

- **Show a status item.** Use `onView:<item>`. The item's label and icon are shown from the
  manifest until then.
- **Watch for something in the terminals.** Use `onSessionEvent:<kind>`.
- **Restore state.** Keep it with `lume.kv` and read it when the code is next started.
- **Set a value once.** If it never changes, it belongs in the manifest.

A status item behind `shown-when` that the user has turned off is not on screen, so its `onView`
never fires and its plugin never starts. A user who turns every meter off pays nothing.

## Idle unload

Lume stops the code of a plugin that has done nothing for five minutes. The code starts again, from
the top of its file, at the next activation event. What the plugin last showed stays on screen in
between.

The code is stopped only when all of this holds:

- the manifest does not list `onStartup`;
- no timer is set;
- no call is waiting for its answer, and no command is being carried out;
- nothing is open: no file watch, no stream, no program it started;
- it listens for no session or workspace events, or the manifest lists an `onSessionEvent` that
  would start it again.

So:

- **A `setInterval` keeps a plugin loaded for ever.** Clear the timer when there is nothing to
  poll, and set it again when there is.
- **A file watch keeps it loaded.** Dispose of watches that are no longer needed.
- **A plugin that calls `lume.sessions.onEvent` should list the kinds it needs as
  `onSessionEvent:<kind>`.** Without them it is never unloaded, because nothing would start it
  again for the next event.

Nothing in memory outlives an unload. Keep what matters with `lume.kv`. `lume.onDeactivate` is
called just before the code goes.

## Stay inside the budgets

Each piece of work has 100 ms, and a turn has 400 ms. See [budgets](environment.md#budgets). A
plugin that goes over is stopped, and three times in a minute turns it off.

- Do the heavy work outside the engine. Running `git` with `lume.process.run` costs the engine
  nothing while the program runs: waiting is free.
- Break long loops up. Work through a large list in pieces, with a timer between them.
- Ask for less. `lume.fs.read` takes `maxBytes`; `lume.process.run` takes `maxOutput`.
- Do not log in a hot path. A log holds 100 lines a second and drops the rest.

## One turn is one step

A turn is one piece of work the code is given, with the promise jobs it leads to. Everything a
plugin shows during one turn appears together, in one step, when the turn is over:

- status values: `lume.status.set`, `lume.status.clear`;
- panel rows: `lume.panels.set`, `lume.panels.clear`, `lume.panels.setChildren`;
- conditions: `lume.context.set`.

A window never shows half of a change: a meter with its new value beside its old colour, or a panel
emptied a moment before it is filled again.

Lume then tells the windows at most ten times a second. Everything a plugin showed since the last
time arrives as one change.

### What shows something is not waited for

The calls that only change what a plugin shows (`lume.status.set` and `clear`, `lume.context.set`,
`lume.panels.set`, `clear` and `setChildren`) are not answered. Their promise is settled at once,
in the same turn, and a call that fails says so in the plugin's log instead (`lume plugin logs`).
Awaiting them is harmless and splits nothing:

```js
// One turn: both change together.
await lume.status.set("cpu", { value: 0.42 });
await lume.status.set("count", { text: "3" });
```

### An `await` on any other call ends the turn

The answer to any other `lume` call arrives as a new piece of work, in a later turn. So does a
timer. Code after such an `await` is in another turn, and what it shows is another step.

This shows two steps, and the user may see the first without the second:

```js
// Two turns: the meter changes, then, once the value has been read, the badge.
lume.status.set("cpu", { value: 0.42 });
const count = await lume.kv.get("count");
lume.status.set("count", { text: String(count) });
```

This shows one:

```js
// One turn: what is needed is read first, then everything is shown together.
const count = await lume.kv.get("count");
lume.status.set("cpu", { value: 0.42 });
lume.status.set("count", { text: String(count) });
```

The rule: gather first, then show. Do every read and every wait, and then make all the calls that
show something one after another, with no `await` on a call between them.

```js
async function refresh() {
  // Gather: as many turns as it takes.
  const [status, branch] = await Promise.all([
    lume.process.run("git", ["status", "--porcelain"]),
    lume.process.run("git", ["branch", "--show-current"]),
  ]);
  const changed = status.stdout.split("\n").filter(Boolean);

  // Show: one turn.
  await Promise.all([
    lume.status.set("branch", { text: branch.stdout.trim() }),
    lume.panels.set(
      "changes",
      changed.map((line, index) => ({ id: String(index), label: line.slice(3) })),
    ),
    lume.context.set("dirty", changed.length > 0),
  ]);
}
```

### Set, do not clear and fill

`lume.panels.set` puts new rows in place of the old ones. There is no need to clear first.

```js
// The panel is empty while the program runs.
await lume.panels.clear("changes");
const rows = await build();
await lume.panels.set("changes", rows);
```

```js
// The old rows stay until the new ones take their place.
const rows = await build();
await lume.panels.set("changes", rows);
```

Build the new rows, then set them. The same goes for status items: `lume.status.set` changes only
the fields it is given and keeps the rest, so there is no reason to clear an item before setting
it.

## Lazy rows

A panel holds 5000 rows at most. A tree larger than that, or one that is slow to build, should not
be built whole. Mark a row `lazy: true` and give its rows when the user opens it.

- A row with `lazy: true` is drawn as one that can be opened, though it has no `children` yet.
- When the user opens it for the first time, `lume.panels.onExpand` hears of it.
- `lume.panels.setChildren(id, path, rows)` gives the rows beneath it.

The manifest:

```toml
schema = 1
id = "acme.tree"
name = "Tree"
version = "1.0.0"
api = "1"
main = "host.js"
activation = ["onView:files", "onCommand:show"]

[[contributes.commands]]
id = "show"
title = "Show the tree"

[[contributes.panels]]
id = "files"
title = "Files"
kind = "list"
area = "right"
```

The code:

```js
// A tree whose folders give their rows when they are opened.
const folder = (id) => ({ id, label: id, lazy: true });

const show = async () => {
  await lume.panels.set("files", [
    folder("src"),
    folder("docs"),
    { id: "README", label: "README" },
  ]);
};

lume.commands.register("show", show);

lume.panels.onExpand("files", (event) => {
  // event.path is the ids from the top down to the row: ["src"], or ["src", "nested"].
  const rows =
    event.path.length === 1
      ? [folder("nested"), { id: "main", label: "main.c" }, { id: "lib", label: "lib.c" }]
      : [{ id: "deep", label: "deep.c" }];
  lume.panels.setChildren("files", event.path, rows).catch((error) => lume.log.error(error));
});

void show();
```

What happens:

1. The panel comes on screen. `onView:files` starts the code, and the last line shows the top
   rows. Two of them are lazy.
2. The user opens `src`. The listener is called with `{ panel: "files", row: "src", path: ["src"] }`.
3. The code answers with three rows, one of them a lazy folder again.
4. The user opens `nested`. The listener is called with `path: ["src", "nested"]`, and answers with
   the rows at that depth.

The details:

- **The event.** `panel` is the panel's name, `row` the id of the row that was opened, and `path`
  the ids of the rows from the top down to it. `path` is what `setChildren` takes.
- **The wait.** While the code has not answered, the row shows that it is waiting. After ten
  seconds it shows that nothing came. Answer every expand, with an empty list if there is nothing.
- **Once.** The listener hears of a row only while it has not been given its rows. Rows given with
  `setChildren` stay until the panel's rows are set anew.
- **Starting.** Opening a row starts the plugin's code if it is not running. A plugin unloaded for
  being idle is started to answer.
- **Limits.** Rows given this way count toward the panel's 5000 rows and its eight levels.
- **A new `set` takes them away.** `lume.panels.set` on the panel replaces everything, the rows
  given with `setChildren` too. To refresh one folder, call `setChildren` for it again.
- **Errors.** `setChildren` fails as `invalid` for a row that is not `lazy`, and as `not_found`
  when the panel shows no rows or has no row at that path.

A plugin that was unloaded and started again has lost what it knew of the tree. Work the rows out
from `event.path` alone, as the example does, and not from state built up in memory.

## Read the machine only while someone looks

`lume.metrics.subscribe(kinds, listener, options)` gives the listener a reading of the machine
about once a second, with the kinds asked for: `cpu`, `memory`, `gpu`, `network`.

Reading the machine is not free. `options.while` names status items of the plugin's own, and the
machine is then read only while one of them is on screen in some window:

```js
let subscription = null;

function apply(settings) {
  const shown = ["cpu", "memory"].filter((item) => settings[item] !== false);
  subscription?.dispose();
  subscription = null;
  if (shown.length > 0) {
    subscription = lume.metrics.subscribe(shown, show, { while: shown });
  }
}

function show(sample) {
  if (sample.cpu !== undefined) {
    lume.status.set("cpu", { value: sample.cpu, text: `${Math.round(sample.cpu * 100)}%` });
  }
  if (sample.memory !== undefined) {
    lume.status.set("memory", { value: sample.memory.used / sample.memory.total });
  }
}

lume.settings.onChange(apply);
lume.settings
  .get()
  .then(apply)
  .catch((error) => lume.log.error(error));
```

- Each name in `while` is a status item the manifest declares.
- A minimised window does not count. With every window minimised, or every item turned off or
  hidden behind `shown-when`, nothing is read.
- Without `while`, the machine is read for as long as the subscription lasts.
- Dispose of the subscription when it is no longer needed.

**A busy plugin gets only the newest reading.** A reading the code has not taken yet is replaced by
the next one. A plugin that falls behind hears the latest when it is free, and never works through
a row of old ones. So do not count readings to measure time, and do not expect one a second.

The listener above makes two `lume.status.set` calls without waiting for either. They are in one
turn, so both meters change together.

Other events are not replaced. They wait in line, 1024 at most, and those that arrive after that
are missed. A listener that takes long falls behind.

## Synchronized output

A program that redraws its whole screen can say when it begins and when it is done, with
synchronized output: it prints `CSI ? 2026 h` before the redraw and `CSI ? 2026 l` after it.

While such a program is drawing, Lume keeps to its last complete screen:

- the window shows the screen the program last finished, and swaps to the new one in one frame when
  the program says it is done;
- `lume.panes.text(pane)` gives the text of the screen as it was when the program began, not half
  of the next one.

This holds for up to 150 ms. A program that does not say it is done in that time is shown and read
as it stands, and is not waited for again until it has ended that screen and begun another. So a
program that never ends its screen cannot freeze a pane.

For a plugin author this means two things:

- Text read with `lume.panes.text` from a program that brackets its redraws is always a whole
  screen. There is no need to read twice and compare.
- A program a plugin ships or drives should bracket its own redraws. It will not flicker, and
  whatever reads its screen will see it whole.

Nothing changes for a program that does not use synchronized output: it is shown and read as it
prints.
