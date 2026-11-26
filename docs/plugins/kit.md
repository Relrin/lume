# The plugin kit

A plugin's panel or pane view is a page of its own, shown in a sandboxed frame. `@lume/plugin-kit`
is what such a page is built with:

- `connect()`, the view's way to its plugin's code and to the window around it;
- hooks that read from it in React;
- controls that look like the window's own and follow its theme.

The kit is a development dependency. A view's script is bundled with everything it imports, the kit
and React among them, into one file in the plugin's package.

## Starting from the template

The lume-addons repository has a template with a panel, a pane view, the plugin's code and the build
for all of it:

```sh
pnpm new notes            # makes plugins/notes, with the id addons.notes
pnpm new notes --id acme.notes --title "Field notes"
pnpm install
pnpm --filter @lume-addons/notes build
lume plugin link plugins/notes
```

What it makes:

```text
notes/
  plugin.toml             the manifest: a frame panel, a pane view, a command
  ui/index.html           the page, which names the two built files
  src/host.ts             the plugin's code      -> dist/host.js
  src/view/main.tsx       the page's script      -> dist/view.js, dist/view.css
  src/view/App.tsx        what the page shows
  src/shared.ts           the names the code and the page call each other by
  vite.config.ts          builds the code
  vite.view.config.ts     builds the page's script and stylesheet
```

## What a page may load

A view's page loads files from its own package and nothing else. It cannot fetch, open a socket,
or load a script, a font or a picture from anywhere else. Its scripts are files named with
`<script src>`: a script written into the page does not run, and neither does `eval`. Styles may be
written inline.

So the page is short, and the build makes one script and one stylesheet for it:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Notes</title>
    <link rel="stylesheet" href="../dist/view.css" />
    <script src="../dist/view.js" defer></script>
  </head>
  <body>
    <div id="root"></div>
  </body>
</html>
```

Build the script as one plain file (Vite's `iife` format), not as modules that import one another.

## `connect()`

```ts
import { connect } from "@lume/plugin-kit";

const view = await connect();
```

The window hands a page its channel once the page has loaded. `connect()` waits for that, says the
view is ready, and gives back the view. Call it as the script starts: a page that has not connected
ten seconds after loading is shown as stopped, with a button to load it again. Calling `connect()`
again gives the same view.

A view that is not written with React imports from `@lume/plugin-kit/bridge`, which is `connect()`
and what it gives, without anything of React's.

| On the view              | What it is                                                                     |
| ------------------------ | ------------------------------------------------------------------------------ |
| `instance`               | The view as the plugin's code knows it: what `lume.views.post` takes.          |
| `plugin`, `view`         | The id the plugin is installed under, and the view's name in the manifest.     |
| `place`                  | `"pane"` or `"panel"`.                                                         |
| `host.call(name, args?)` | Calls the plugin's code. Settles with its answer, or fails with a `HostError`. |
| `host.on(event, fn)`     | Hears an event from the plugin's code. Returns what stops listening.           |
| `state.get()`            | What is kept for the view: what it last set, or what it started with.          |
| `state.set(value)`       | Keeps a value for the view's next start.                                       |
| `streams.open(grant)`    | Opens a stream the plugin's code let this view use.                            |
| `theme.current()`        | The window's look now: `{ tokens, scheme }`.                                   |
| `theme.onChange(fn)`     | Hears the look change.                                                         |
| `onFocus(fn)`            | Hears the keyboard come to the view.                                           |
| `onRefused(fn)`          | Hears the window refuse something the view sent.                               |
| `passKey(key)`           | Passes a key on to the window.                                                 |
| `fail(message)`          | Says the view failed. The window shows it as stopped.                          |

### Calls and events

The plugin's code answers a view's calls with `lume.views.handle`, and reaches its views with
`lume.views.post`:

```ts
// The plugin's code.
lume.views.handle("add", async (args, from) => {
  const next = (await count()) + 1;
  await lume.kv.set("count", next);
  await lume.views.post(from.view, "count", next);
  return next;
});
```

```ts
// The view.
const next = await view.host.call<number>("add");
const stop = view.host.on<number>("count", (count) => show(count));
```

Arguments, answers and payloads are JSON, of 1 MiB at most. A call that fails rejects with a
`HostError`, whose `code` is the kind of failure: one of the application's (`denied`, `not_found`,
`invalid`, `busy`, `failed` and the rest), or the plugin's own.

A plugin names its own kinds of failure. Its code throws a `LumeError` with a code of its own and,
if it likes, something to say of it; the view finds both on the error it catches:

```ts
// The plugin's code.
lume.views.handle("merge", async (args) => {
  const conflicts = await merge(args.branch);
  if (conflicts.length > 0) {
    throw new lume.LumeError("conflict", "The branches conflict.", { files: conflicts });
  }
});

// The view.
try {
  await view.host.call("merge", { branch });
} catch (error) {
  if (error instanceof HostError && error.code === "conflict") {
    showConflicts((error as HostError<{ files: string[] }>).data?.files ?? []);
  } else {
    throw error;
  }
}
```

See [errors of the plugin's own](environment.md#errors-of-the-plugins-own). A view waits for 64 answers at most at a time, and may send some 200 messages a second; a
view that sends far more than that is stopped.

### What is kept

`state.set(value)` keeps a value for the view: JSON of 64 KiB at most. The view finds it again with
`state.get()` whenever it starts: after the application was closed and opened, after its pane was
moved to where its frame could not follow, and after it was out of sight for half a minute and let
go of. A view opened for a file or an address starts with `{ open: { path } }` or
`{ open: { uri } }` as its state.

Keep what the user would miss: the file shown, the tab chosen, a draft. What the plugin's code can
give again is better asked for again.

### Streams

For more than a few messages' worth of bytes, a file or a program's output, the plugin's code hands
the view a stream instead of passing the bytes on itself:

```ts
// The plugin's code.
lume.views.handle("log", async (_args, from) => {
  const child = await lume.process.spawn("git", ["log", "--oneline"]);
  return lume.streams.grantToView(from.instance, child.stdout);
});
```

```ts
// The view.
const stream = await view.streams.open(await view.host.call<string>("log"));
const decoder = new TextDecoder();
stream.onData((piece) => append(decoder.decode(piece, { stream: true })));
const end = await stream.ended; // { ok, bytes }
```

A grant is for one view and is opened once. A stream has `readable` and `writable`, `onData(fn)`,
`write(bytes | text)`, `finish()` (nothing more will be written), `close()` and `ended`. No more
than about a megabyte is on its way to a view at a time: a piece counts as taken when the listener
has returned, or when the promise it returned has settled, so a slow view holds the stream back
instead of filling up.

### Keys and the keyboard

While the keyboard is in a view, the window sees none of its keys. `connect()` passes on the keys
the view did not use, so that the keys that move about the window still work from inside it: those
pressed with Ctrl, Alt or the logo key, and the function keys. A key the view handled with
`preventDefault()` is not passed on, and neither are the keys a text field uses itself (copy,
paste, undo, moving by words) while the keyboard is in one.

The window runs only a fixed few commands from such keys: moving to another pane or tab, the
palette, a side panel, the bar. Nothing a view passes on can type, paste, close or start anything.

When the view's pane is focused, the view is told. The kit then focuses the element marked
`autofocus`; a view that wants something else listens with `onFocus`.

`connect({ forwardKeys: false })` turns the passing on off, for a view that does it itself with
`passKey`.

## The window's look

The window tells a view its design tokens when the view starts and whenever they change: a CSS
variable for every colour, size and font the window is drawn with. `connect()` sets them on the
page's root element, so styles written with them follow the theme without more:

```css
.note {
  background: var(--lume-color-card);
  border: 1px solid var(--lume-color-border);
  border-radius: var(--lume-radius-control);
  color: var(--lume-color-text);
}
```

The root element also carries `data-scheme="light"` or `"dark"`.

The names are those of the theme's tokens with `--lume-` before them and dashes for dots:
`--lume-color-surface`, `-card`, `-border`, `-text`, `-text-muted`, `-accent`, `-accent-text`,
`-control`, `-control-hover`, `-control-active`, `-popover`, `-status-info`, `-status-success`,
`-status-warning`, `-status-danger`; `--lume-font-ui-family`, `--lume-font-ui-size`,
`--lume-font-family` (the terminal's); `--lume-radius-control`, `--lume-radius-card`;
`--lume-space-gap`; `--lume-shadow-popover`; `--lume-motion-duration-fast`, and the rest of the
set.

A font is a name: a view is told which font the window uses, not given its file. A font the system
does not have falls back as it would on any page, unless the plugin ships the file itself.

Import the kit's stylesheet once, before the view's own:

```ts
import "@lume/plugin-kit/kit.css";
```

It sets the page's background, text and font from the tokens and styles the controls below.
`connect({ applyTokens: false })` leaves the page alone, for a view that sets the tokens itself
with `applyTokens(tokens, scheme, element)`.

## React

```tsx
import "@lume/plugin-kit/kit.css";

import { connect, ViewProvider } from "@lume/plugin-kit";
import { createRoot } from "react-dom/client";

void connect().then((view) => {
  createRoot(document.getElementById("root")!).render(
    <ViewProvider view={view}>
      <App />
    </ViewProvider>,
  );
});
```

Inside the provider:

| Hook                        | What it gives                                                                  |
| --------------------------- | ------------------------------------------------------------------------------ |
| `useView()`                 | The connected view.                                                            |
| `useHostEvent(event, fn)`   | Calls `fn` for an event from the plugin's code, while the component is shown.  |
| `useViewState(initial)`     | `[value, set]`, like `useState`, kept for the view's next start.               |
| `useStream(grant, onData?)` | Opens a grant's stream while the component is shown, and says where it stands. |
| `useTheme()`                | `{ tokens, scheme }`, followed as the look changes.                            |

`useViewState` holds the view's whole state, so use it once, with an object in it:

```tsx
const [kept, setKept] = useViewState({ tab: "changes", filter: "" });
```

`useStream` takes `null` until there is a grant, and gives `{ status }`: `idle`, `opening`, `open`
(with `stream`), `ended` (with `end`) or `failed` (with `error`).

## Controls

Each control is the plain HTML control it looks like and takes that control's attributes.

**`Button`** has words on it. `variant` is `default` (outlined), `primary` (the one thing to do
next), `quiet`, `accept` or `danger`; `size` is `medium` or `small`.

```tsx
<Button variant="primary" onClick={commit}>
  Commit
</Button>
```

**`IconButton`** is a square button with a picture: an inline SVG drawn with `currentColor`. It
needs a `label`; `pressed` makes it a button that stays down.

```tsx
<IconButton label="Next file" onClick={next}>
  <ArrowDown />
</IconButton>
```

**`SplitButton`** is a button with its other ways in a menu under an arrow beside it.
**`MenuButton`** is a button that only opens a menu.

```tsx
<SplitButton
  variant="primary"
  onClick={commit}
  menuLabel="More ways to commit"
  items={[
    { id: "push", label: "Commit and push" },
    { id: "amend", label: "Amend last commit" },
    { separator: true },
    { id: "undo", label: "Undo last commit", danger: true },
  ]}
  onSelect={run}
>
  Commit
</SplitButton>
```

**`Segmented`** is a few choices side by side with one taken.

```tsx
<Segmented
  label="Layout"
  value={layout}
  onChange={setLayout}
  options={[
    { value: "stacked", label: "Stacked" },
    { value: "split", label: "Split" },
  ]}
/>
```

**`Switch`** is on or off and changes at once. **`Checkbox`** has three states: `true`, `false`
and `"mixed"`, for a heading over rows of which only some are ticked.

```tsx
<Switch checked={wrap} onChange={setWrap} label="Wrap lines" />
<Checkbox checked={staged} onChange={stageAll} label="Stage all" />
```

**`Tabs`** go across the top of a panel. What each tab shows is the view's to draw.

```tsx
<Tabs
  label="Git"
  value={tab}
  onChange={setTab}
  tabs={[
    { id: "changes", label: "Changes", count: 4 },
    { id: "history", label: "History" },
  ]}
/>
```

**`Badge`** is a word or a number in a small box. `tone` is `neutral`, `info`, `success`,
`warning`, `danger` or `accent`; `outline` draws it outlined.

```tsx
<Badge tone="info">Pending</Badge>
```

**`TextArea`** is a field for several lines; `mono` draws it in the terminal's font.

**`Popover`** is a small box of controls that opens beside its `anchor`, below or above it
(`side`), lined up with its left or right edge (`align`). A press outside it or Escape closes it. A
view cannot draw outside its own frame, so a popover stays inside the view. **`Menu`** is the rows
of a menu, for a popover of the view's own.

```tsx
<Popover
  open={open}
  onClose={() => setOpen(false)}
  label="Display"
  anchor={<Button onClick={() => setOpen(!open)}>Display</Button>}
>
  …
</Popover>
```

**`List`** is rows with one selected; the arrow keys move the selection, and Enter or a double
click activates a row. **`VirtualList`** is the same for thousands of rows of one height: it draws
only those in sight, and fills the box it is put in.

```tsx
<VirtualList
  label="History"
  items={commits}
  rowHeight={30}
  getKey={(commit) => commit.hash}
  selected={hash}
  onSelect={setHash}
  renderRow={(commit) => <CommitRow commit={commit} />}
/>
```

**`Tree`** is folders and what is in them. The view keeps which nodes are open. Right opens a node,
Left closes it or goes to its parent. With a `rowHeight` it draws only the rows in sight.

```tsx
<Tree
  label="Files"
  nodes={nodes} // { id, data, children? }[]
  expanded={open}
  onToggle={(id, isOpen) => setOpen(toggled(open, id, isOpen))}
  selected={path}
  onSelect={setPath}
  renderRow={(file, row) => <FileRow file={file} folder={!row.leaf} />}
/>
```

**`SplitView`** is two parts with a line between them that is dragged, or moved with the arrow
keys. The view keeps the share.

```tsx
<SplitView share={share} onShareChange={setShare} first={<Files />} second={<Diff />} />
```

The class names in `kit.css` start with `lk-`. `lk-scroll` gives a box a thin scroll bar, `lk-mono`
the terminal's font and `lk-muted` the quieter text colour.
