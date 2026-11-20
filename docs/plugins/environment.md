# The JavaScript environment

A plugin's code runs in a sandboxed JavaScript engine. Each plugin has an engine of its own, on a
thread of its own, and shares nothing with any other plugin or with Lume's windows.

## What exists

- The JavaScript standard library: `Promise`, `Map`, `JSON`, `Math`, typed arrays and the rest of
  the language.
- The global `lume` object. See the [API reference](api.md).
- The global `console`, with `log`, `info`, `debug`, `warn` and `error`. Each writes to the
  plugin's log.

Both globals are frozen. Nothing can be added to them or changed on them.

## What does not exist

| Missing                                            | Use instead                                         |
| -------------------------------------------------- | --------------------------------------------------- |
| `setTimeout`, `setInterval` as globals             | `lume.timers.setTimeout`, `lume.timers.setInterval` |
| `fetch`, `XMLHttpRequest`, `WebSocket`             | `lume.net.fetch`, with the `net` permission         |
| Node built-ins: `require`, `process`, `fs`, `path` | `lume.fs`, `lume.process`, with their permissions   |
| The DOM: `window`, `document`, elements, styles    | Nothing. Lume draws what the plugin shows.          |
| Modules: `import`, `export`, a module loader       | Bundle the code into one script.                    |

The code has no files, no network and no processes of its own. Everything it does beyond its own
memory is a call to `lume`, which Lume checks against what the plugin was granted. See
[Permissions](permissions.md).

## How the code runs

The file `main` names is run once, from top to bottom, as a plain script. That is the plugin
starting. Register command handlers, services and listeners there, before the code waits for
anything.

After that the code is given one piece of work at a time:

- the answer to a call it made;
- one of its commands to carry out;
- a call of one of its services;
- an event it listens for;
- a timer that fell due.

Each piece of work runs together with the promise jobs it leads to. Together they are one turn.
Calls the code makes during a turn are handed to Lume when the turn is over, so nothing of Lume's
runs in the middle of the code's work. What a turn shows appears together: see
[Performance](performance.md#one-turn-is-one-step).

The code has 10 seconds to start, and 30 seconds to carry out a command or answer a call of a view
or of a service. After that the command or the call fails as timed out. The time the user is being
asked something is not counted: a prompt for a permission, the approval of a use, a `ui.confirm` or
a `ui.pick`. So a handler may ask first and then do its work. Work that is slow by itself, like
talking to a server, should answer at once and say how it went with an event.

## The parts of `lume` that are not plain calls

Most of the API is calls that return promises. The [API reference](api.md) lists them. These parts
are different, and the typings describe them:

| Part                                         | What it does                                                                        |
| -------------------------------------------- | ----------------------------------------------------------------------------------- |
| `lume.self`                                  | What the plugin is: `id`, `manifestId`, `name`, `version`.                          |
| `lume.commands.register(name, handler)`      | Carries out one of the plugin's commands.                                           |
| `lume.services.export(name, handler)`        | Offers one of the plugin's services to other plugins.                               |
| `lume.timers`                                | `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`.                       |
| `lume.sessions.onEvent(listener)`            | Hears what sessions report. Needs `workspace.read`.                                 |
| `lume.panes.onEvent(listener)`               | The same as `lume.sessions.onEvent`.                                                |
| `lume.workspace.onChange(listener)`          | Hears when the workspace in use or its root folder changes. Needs `workspace.read`. |
| `lume.fs.watch(path, listener, options?)`    | Watches a file or folder. Needs `fs.read` over it. Returns a promise.               |
| `lume.settings.onChange(listener)`           | Hears when the plugin's settings change. Given all of them.                         |
| `lume.panels.onExpand(id, listener)`         | Hears when the user opens a lazy row of a panel.                                    |
| `lume.metrics.subscribe(kinds, listener, o)` | Gives each reading of the machine. Needs `system.metrics`.                          |
| `lume.log`                                   | `debug`, `info`, `warn`, `error`: writes to the plugin's log.                       |
| `lume.onDeactivate(fn)`                      | Calls `fn` when the code is about to be stopped.                                    |
| `lume.LumeError`                             | The class of the errors calls fail with.                                            |

`register`, `export` and every `on...` and `subscribe` return an object with `dispose()`, which
undoes them. `lume.fs.watch` returns a promise of one. The timer functions return a number for
`clearTimeout` and `clearInterval`.

`lume.self.id` is the id the plugin is installed under. It may not be the id in its manifest, which
is `lume.self.manifestId`. Never write the plugin's id into the code.

A command's handler is given the command's arguments, or `null`. What it returns must be JSON. A
service's handler is given the caller's arguments, or `null`, and what it returns is given back to
the caller. Either may be `async`.

## Budgets

The code is held to limits, so a plugin cannot stall the terminal or take the machine's memory.

| Limit                       | Value                                                          |
| --------------------------- | -------------------------------------------------------------- |
| Time for one piece of work  | 100 ms. Each promise job after it has 100 ms too.              |
| Time for one turn           | 400 ms: a piece of work with all the promise jobs it leads to. |
| Promise jobs in one turn    | 10 000.                                                        |
| Memory                      | 64 MiB, or what `[limits] memory` says: from 16 to 256 MiB.    |
| Stack                       | 1 MiB.                                                         |
| Calls waiting for an answer | 256.                                                           |
| Timers set                  | 1000. The shortest a timer can be is 4 ms.                     |
| Events waiting for the code | 1024.                                                          |

What happens at each limit:

- **Time.** The engine is interrupted when a piece of work runs over its 100 ms, or a turn runs
  over 400 ms or 10 000 jobs. A chain of promises that never ends is stopped this way too. Work the
  interrupt cannot reach is watched from outside, and a plugin still busy after 500 ms is given up
  on. Each of these is a breach.
- **Memory.** The plugin is stopped when it runs out of its memory, even if its code catches the
  error, and when it is using nine tenths of its memory at the end of a turn. Both are breaches.
- **Calls.** A call made while 256 wait for an answer fails at once with the code `busy`. This is
  not a breach.
- **Timers.** A timer set while 1000 are set is not set, and a warning is written to the log. A
  timer shorter than 4 ms waits 4 ms.
- **Events.** An event that arrives while 1024 wait for the code is missed.

Waiting is free. Time is counted only while the code runs, so an `await` on a call that takes a
second costs nothing.

### A breach

On a breach the engine is thrown away. Whatever the code held in memory is gone, calls it was
waiting on are dropped, and what it had open (streams, watches, programs it started) is closed. The
user sees a notice that the plugin was stopped, and a line is written to its log.

The plugin starts again, from the top of its file, the next time something wants it: a command, a
service call, an activation event.

Three breaches in a minute turn the plugin off. The user is told, and turns it on again in the
settings or with `lume plugin enable <id>`.

## Logs

`lume.log.debug`, `info`, `warn` and `error` each write one line to the plugin's log. `console.log`
and `console.info` write at `info`; `console.debug`, `console.warn` and `console.error` at their
own level. Values that are not strings are written as JSON. An `Error` is written with its message
and its stack.

A line looks like this:

```text
2026-11-19T08:30:05Z info greeted 1
```

Read the log with `lume plugin logs <id>`, which shows the last 40 lines, or `--lines <n>` of them.
The settings window shows the end of it under **Add-ons**.

Lume writes to the same log itself:

- an error the code threw and did not catch, with its stack;
- a promise that was rejected with nothing handling it, as a warning;
- a breach, as an error that begins `stopped:`.

Limits on the log:

- One message is 8 KiB at most. A longer one is cut.
- A plugin writes 100 lines a second at most. The rest are left out, and one line says how many.
- When the log passes 1 MiB it is set aside and a new one begun.

## Source maps

A stack names lines of the bundle, which say little when the plugin was written in TypeScript and
bundled. Ship the bundle's source map in the package as `<main>.map`: for
`main = "dist/host.js"`, the file `dist/host.js.map`. Stacks in the log are then written with the
source's files and lines. The map is read only when a stack first needs it, so it costs nothing
when the code starts.

## Errors

A call that fails rejects its promise with a `LumeError`. It is an `Error` with a `code` that says
what kind of failure it was, and a `message` for the log.

```js
try {
  await lume.fs.read("${workspace}/notes.txt");
} catch (error) {
  if (error instanceof lume.LumeError && error.code === "not_found") {
    // There is no such file.
  } else {
    throw error;
  }
}
```

| Code          | Meaning                                                                |
| ------------- | ---------------------------------------------------------------------- |
| `denied`      | The plugin has no permission for it, or the user said no.              |
| `invalid`     | What was sent is not what the call takes.                              |
| `not_found`   | What was asked for is not there.                                       |
| `busy`        | The plugin is making more calls than it may at once.                   |
| `failed`      | It could not be done.                                                  |
| `stopped`     | The plugin was stopped before it was done.                             |
| `unhandled`   | The command was not handled by the plugin's code.                      |
| `too_large`   | What was asked for, or what came back, is larger than the call allows. |
| `unavailable` | What the call needs is not there on this machine, such as a keychain.  |
| `timed_out`   | It took longer than it may.                                            |

### Errors of the plugin's own

The codes above are the application's, for what the application's calls can fail of. A plugin has
kinds of failure of its own, and names them itself: it throws a `LumeError` with its own code and,
if it likes, something to say of the failure.

```js
lume.views.handle("checkout", async (args) => {
  const dirty = await changedFiles();
  if (dirty.length > 0) {
    throw new lume.LumeError("uncommitted", "There are changes that are not committed.", {
      files: dirty,
    });
  }
  // ...
});
```

Whoever called finds both on the error it catches: `error.code === "uncommitted"`, and
`error.data.files`. That is so for a view calling the plugin's code and for another plugin calling
one of its services. Lume carries the code and the data as they are and gives them no meaning, so
a plugin adds a kind of failure without anything in Lume changing.

- A code of the plugin's own is lower-case letters, digits, `_`, `-` and `.`, beginning with a
  letter, of 64 characters at most. Anything else fails as `failed`, with its message.
- `data` is JSON of 16 KiB at most. More than that is left out; the failure and its message still
  arrive.
- One of the application's codes may be thrown too, as in `new lume.LumeError("not_found", …)`,
  and may carry `data`.
- The message is for people and for the log. Have callers decide by the code, never by the
  message.

What happens to an error the code throws:

- **In the top level of the file.** The plugin does not start. The error is logged with its stack,
  and the user sees a notice that the plugin could not start.
- **In a command's or a service's handler.** The error is logged, and the command or the call fails
  with the error's message. A `LumeError` keeps its code and its data; anything else fails as
  `failed`.
- **In a listener or a timer's function.** The error is logged, and the plugin goes on.

A command the manifest declares but the code registered no handler for fails as `unhandled`.

## Stopping

Lume stops a plugin's code when it has been idle for a while (see
[Performance](performance.md#idle-unload)), when the plugin is turned off, removed or replaced by
a package with other contents, and on a breach.

`lume.onDeactivate(fn)` registers a function Lume calls just before it stops the code. It is not
called on a breach.

```js
lume.onDeactivate(() => subscription?.dispose());
```

The function runs to its end and the engine goes. Nothing can be waited for there: a call made from
it is rejected as `stopped`, like every call still waiting for its answer. After it, the plugin's
timers, listeners, handlers and watches are all gone.

Nothing in memory outlives a stop. Keep what must last with `lume.kv` (JSON values, 1 MiB together,
keys of 1 to 256 characters), in files under `${plugin-data}` with `fs.write` over them, or in the
system's keychain with `lume.secrets`.
