# Views

A plugin can show pages of its own: in a panel, or in a pane where a terminal would be. Such a
page is a **view**. It is an HTML file in the plugin's package, shown in a sandboxed frame, and it
can reach one thing: its own plugin's code.

Everything else a plugin shows (status items, list panels, menus) is drawn by Lume from data, and
needs no page. Use a view when the plugin needs to draw for itself.

Most views are built with [the plugin kit](kit.md), which does what this page describes. Read this
page to know what a view can and cannot do, and to write one without the kit.

## Declaring a view

A view is declared in the manifest as a panel of kind `frame` or as a pane view, with the page as
its `entry`:

```toml
schema = 1
id = "acme.review"
name = "Review"
version = "1.0.0"
api = "1"
main = "dist/host.js"
activation = ["onView:changes", "onView:diff", "onCommand:open"]

[[contributes.commands]]
id = "open"
title = "Open the Review"

# A page in a dock area of the window.
[[contributes.panels]]
id = "changes"
title = "Changes"
kind = "frame"
area = "right"
entry = "ui/changes.html"

# A page in a pane.
[[contributes.pane-views]]
id = "diff"
title = "Diff"
entry = "ui/diff.html"

# Files and addresses the pane view opens.
[[contributes.openers]]
view = "diff"
title = "Review"
patterns = ["*.diff", "*.patch"]
schemes = ["acme-review"]
```

See [the manifest](manifest.md#contributespane-views) for every key. `onView:<name>` starts the
plugin's code when the view is first shown; a view whose plugin has code is never shown without it.

A panel is shown and hidden by the user, like any panel. A pane view is opened:

- by the plugin's code, with [`lume.ui.openView`](api.md#lumeuiopenviewview-options), which needs
  no permission for the plugin's own views;
- by the command `lume.view.open`, with `{ "plugin": …, "view": … }`;
- by an opener, when the user opens a file or an address it matches.

## A pane's picture

A pane that shows a view has the view's `icon` in its header, in its tab and in its marker when it
is put away, where a terminal has the tile of the program it runs. A view with no `icon` shows a
terminal's prompt.

A plugin with the `layout.write` permission can give any pane a picture to be known by, a
terminal it opened as well as a view, and change it as what the pane is for changes:

```js
await lume.layout.apply({ type: "pane_icon", pane, icon: "report" });
```

- `icon` is one of the plugin's own [pictures](motion.md#pictures), by its name, or one of the
  application's tiles, by the name `appearance.program-icons` takes for it (`agent`, `editor`,
  `git`, a program's own such as `docker`).
- `icon: null` takes the picture away: the pane shows its view's, or what runs in it, again.
- The picture is kept with the workspace. One that names nothing there is, as after its plugin is
  removed, is passed over.
- A picture of the plugin's own that is `tint`ed is drawn in the tile's ink; any other keeps its
  colours.

## What a page may load

The frame is sandboxed with scripts allowed and nothing else. Its origin matches no other page's,
so:

- it has no cookies, no `localStorage` and no IndexedDB;
- it cannot reach the window's page, another plugin's frame or another of its own;
- it cannot navigate the window, open windows or submit forms. A frame that loads a second page is
  taken down.

Every file is served with a policy that holds the page to its own package:

| What                               | From where                                                                                    |
| ---------------------------------- | --------------------------------------------------------------------------------------------- |
| scripts                            | files of the plugin's package only: none inline, no `eval`                                    |
| styles                             | files of the package, and inline styles                                                       |
| pictures, fonts, media             | files of the package; also `data:` for pictures and fonts, and `blob:` for pictures and media |
| workers                            | files of the package, and `blob:`                                                             |
| `fetch`, WebSocket, any connection | nothing                                                                                       |
| frames                             | none                                                                                          |

Scripts must be plain files loaded with `<script src>`: the page's origin matches nothing, so a
script of `type="module"`, which is fetched under rules a matchless origin cannot meet, does not
load. Bundle the view into one script (the kit's template does).

Only files inside the package are served, by paths that stay inside it. The page itself is checked
against what was installed before it is served, as code is before it starts.

## Connecting

When the frame has loaded, the window posts it one message, with one `MessagePort`:

```js
window.addEventListener("message", (event) => {
  if (event.data?.lume !== "connect" || event.ports.length !== 1) return;
  const hello = event.data;
  const port = event.ports[0];
  port.onmessage = (message) => handle(message.data);
  port.postMessage({ t: "ready" });
});
```

The message says who the view is and how the window looks:

| Key        | What it is                                                                      |
| ---------- | ------------------------------------------------------------------------------- |
| `lume`     | `"connect"`.                                                                    |
| `api`      | The version of this contract: `1`.                                              |
| `instance` | What tells this open view from every other. The plugin's code knows it by this. |
| `plugin`   | The id the plugin is installed under.                                           |
| `view`     | The view's name in the manifest.                                                |
| `place`    | `"pane"` or `"panel"`.                                                          |
| `state`    | What the view last asked to have kept, or `null`.                               |
| `tokens`   | The window's look: every design token's CSS variable and its value.             |
| `scheme`   | `"light"` or `"dark"`.                                                          |

The port is the view's only way out. The view must post `{ t: "ready" }` within ten seconds, or it
is shown as stopped, with a button to load it again.

## Messages

What a view may send on its port:

| Message                         | What it does                                                 |
| ------------------------------- | ------------------------------------------------------------ |
| `{ t: "ready" }`                | Says it is set up and listening.                             |
| `{ t: "call", id, name, args }` | Calls its plugin's code. `id` is a number of the view's own. |
| `{ t: "state", state }`         | Asks to have `state` kept for it.                            |
| `{ t: "key", key, code, mods }` | Passes on a key it did not use.                              |
| `{ t: "stream", id, grant }`    | Opens a stream its plugin's code let it use.                 |
| `{ t: "error", message }`       | Says it failed. It is shown as stopped.                      |

What it is sent:

| Message                                         | When                                                  |
| ----------------------------------------------- | ----------------------------------------------------- |
| `{ t: "result", id, value }`                    | A call was answered.                                  |
| `{ t: "error", id, code, message, data? }`      | A call or a stream failed.                            |
| `{ t: "opened", id, port, readable, writable }` | A stream was opened; `port` is the stream's own port. |
| `{ t: "event", event, payload }`                | Its plugin's code posted it an event.                 |
| `{ t: "tokens", tokens, scheme }`               | The window's look changed.                            |
| `{ t: "focus" }`                                | The keyboard came to the view.                        |
| `{ t: "refused", what, message }`               | Something it sent was not taken.                      |

Limits, for each view:

- 200 messages a second, with up to 400 at once after a quiet spell;
- 1 MiB a message as JSON, 4 MiB a second, with up to 2 MiB at once;
- 64 calls waiting for their answers at a time.

A message over a limit is dropped and `refused` says so. A view that keeps sending past its rate,
or sends something that is none of the messages above, is stopped. Messages are JSON; bytes travel
on [streams](#streams).

## Calling the plugin's code

The code answers calls by name:

```js
// In the plugin's code.
lume.views.handle("files", async (args, from) => {
  // from = { instance, view, place, pane? }
  return listFiles(args.folder);
});
lume.views.onOpen((view) => lume.log.info(`${view.view} opened`));
lume.views.onClose((view) => lume.log.info(`${view.view} closed`));

// To one open view, or to every open view of a name.
await lume.views.post("changes", "refreshed", { count: 3 });
```

A call waits for its answer for 30 seconds, not counting the time the user is being asked
something (a prompt, a confirmation, a choice). Events reach a view in the order they were posted.

A failure of the code comes back as `error`. Its `code` is one of the application's codes, or the
name the plugin's code gave the failure when it threw a `LumeError` of its own; `data` is what the
code said of it, and is left out when it said nothing. A plugin defines its own kinds of failure
this way, and Lume carries them without knowing any. See
[errors of the plugin's own](environment.md#errors-of-the-plugins-own).

The view has no permissions of its own. Whatever it needs done (a file read, a program run) it
asks its plugin's code for, and the code does it within what the plugin was granted. Treat what a
view sends as the code treats anything from outside: check it.

## State

A view starts again more often than a plugin's author expects (see
[when a view starts again](#when-a-view-starts-again)). What it wants to find again it asks to have
kept:

```js
port.postMessage({ t: "state", state: { file: "src/main.rs", scroll: 420 } });
```

- The state is JSON of 64 KiB at most. A larger one is `refused` and the one before stays.
- The last state is handed back in `state` of the connect message.
- A pane view's state is saved with the workspace, so it is there after Lume is closed and opened
  again. Set `persist = false` on the pane view to keep it only while Lume runs.
- A view opened with a state (`lume.ui.openView(view, { state })`, or by an opener) finds that
  state the first time.

A view opened by an opener starts with `{ "open": { "path": … } }` or `{ "open": { "uri": … } }`.

## The window's look

`tokens` holds every design token as a CSS variable name and its value, such as
`--lume-color-surface`. Set them on the page and style with them, and the view follows the user's
theme:

```js
for (const [name, value] of Object.entries(hello.tokens)) {
  document.documentElement.style.setProperty(name, value);
}
```

A `tokens` message comes whenever the theme or a setting that changes the look does.

## Keys

While the keyboard is in a view, the window sees none of its keys. So that the keys that move
about the window still work from inside a view, a view passes on the keys it has no use for: those
pressed with Ctrl, Alt or the logo key, and the function keys.

```js
window.addEventListener("keydown", (event) => {
  if (event.defaultPrevented) return;
  const mods = ["ctrl", "alt", "shift", "meta"].filter((mod) => event[`${mod}Key`]);
  port.postMessage({ t: "key", key: event.key, code: event.code, mods });
});
```

A key passed on is looked up in the user's bindings, and runs its command only if the command
moves the keyboard or shows something of the window's: focus another pane, go to another tab, open
the palette, toggle a panel, focus the bar. Nothing that types, pastes, copies, closes or starts
anything runs from a key a view passed on, whatever it is bound to.

`{ t: "focus" }` tells the view the keyboard has come to it; put the focus where typing should go.

## Streams

Bytes do not travel as messages. For a program's output, a large file or a connection, the plugin's
code hands the view a stream it holds:

```js
// In the plugin's code.
lume.views.handle("log", async (_args, from) => {
  const child = await lume.process.spawn("git", ["log", "--oneline"]);
  return { grant: await lume.streams.grantToView(from.instance, child.stdout) };
});
```

The view opens the grant and gets a port of the stream's own:

```js
// In the view: after { t: "stream", id, grant } was answered by { t: "opened", id, port, … }.
port.onmessage = (event) => {
  const data = event.data;
  if (data.d) {
    use(new Uint8Array(data.d));
    port.postMessage({ ack: data.d.byteLength });
  } else if (data.end) {
    finished(data.ok);
  }
};
```

| On the stream's port | Direction | What it is                                                  |
| -------------------- | --------- | ----------------------------------------------------------- |
| `{ d: ArrayBuffer }` | to view   | A piece of the stream.                                      |
| `{ end: true, ok }`  | to view   | The stream is over.                                         |
| `{ ack: n }`         | from view | The view has taken `n` bytes, which lets as many more come. |
| `{ w: ArrayBuffer }` | from view | Bytes to write, 64 KiB at most in one message.              |
| `{ fin: true }`      | from view | The view will write no more.                                |
| `{ close: true }`    | from view | The view is done with the stream.                           |

- No more than 1 MiB of a stream is on its way to a view or waiting in it. A view that does not
  acknowledge what it was sent is sent no more, so a slow view slows its stream and nothing else.
- A grant is for one open view of the plugin and is opened once. Another view, or another plugin's,
  gets `not_found`.
- A grant ends when the view closes, when the code takes it back with `lume.streams.revoke`, when
  the stream ends, or when the plugin's code stops.

## When a view starts again

A view's frame is started again, from the state it kept, when:

- Lume is started again;
- its pane is floated, docked back, or minimised and brought back;
- its panel's side is folded away and opened again;
- it was out of sight for more than 30 seconds (it keeps running until then, so going to another
  tab and back finds it as it was);
- the user presses **Load it again** on a view that stopped;
- the plugin is being worked on and its files changed (see [developing](develop.md)).

Showing another tab, zooming a neighbour and moving the pane among the tabs keep it running.

The plugin's code outlives its views, and one run of the code may see a view close and open many
times. Keep in the code what belongs to the plugin, and in the view's state what belongs to the
view: what is selected, where it is scrolled to.

## When a view is stopped

A view is shown as stopped, with the reason and a button to load it again, when:

- it did not say `ready` within ten seconds;
- it sent `{ t: "error" }`;
- it tried to leave its page;
- it kept sending past its limits, or sent something that is no message.

A view that keeps the processor busy slows only itself: its frame runs apart from the window's
page.
