# The manifest

`plugin.toml` sits at the root of a plugin's package. It says what the plugin is, what it adds and
what it needs. Lume shows everything a plugin adds from the manifest alone, and starts the plugin's
code only when one of the events the manifest names happens.

The manifest is strict:

- A key Lume does not know is an error, at every level. The message gives the line and the column.
- Keys are written in kebab-case: `slot-setting`, `shown-when`, `setting-groups`.
- Every problem found is reported, each beginning with the key it is about, as in
  `contributes.status[0].slot: ...`.

The JSON Schema of the manifest is in the Lume repository as `schemas/plugin.schema.json`, for
editors that complete TOML from a schema.

## A minimal manifest

```toml
schema = 1
id = "acme.metrics"
name = "Metrics"
version = "0.1.0"
api = "1"
```

## Top-level keys

| Key           | Type             | Needed | What it is                                                                                                                                      |
| ------------- | ---------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `schema`      | integer          | yes    | The version of the manifest's format. It is `1`.                                                                                                |
| `id`          | string           | yes    | The plugin's id. See [ids and names](#ids-and-names).                                                                                           |
| `name`        | string           | yes    | What the plugin is called. One line, 64 characters at most, not empty.                                                                          |
| `version`     | string           | yes    | A full semantic version, as in `1.2.0`. `1.2` is refused.                                                                                       |
| `api`         | string or number | yes    | The major version of the plugin API the plugin is written for: `"1"`.                                                                           |
| `description` | string           | no     | One line, 300 characters at most.                                                                                                               |
| `icon`        | string           | no     | A picture file in the package: `.svg`, `.png`, `.jpg`, `.jpeg`, `.webp` or `.gif`.                                                              |
| `license`     | string           | no     | The plugin's licence.                                                                                                                           |
| `repository`  | string           | no     | Where the plugin's source is. Two packages with the same id and repository are one plugin at two versions. See [updating](install.md#updating). |
| `main`        | string           | no     | The plugin's code: a `.js` file in the package.                                                                                                 |
| `activation`  | list of strings  | no     | The events that start the code. See [activation events](#activation-events).                                                                    |

`main` is needed when the plugin has commands, services, panels or activation events. A plugin that
only adds themes, keys, or menu items that run the application's commands needs no code.

The tables `[permissions]`, `[limits]`, `[dependencies]`, `[load]` and `[contributes]` are
described below.

### Paths in the package

Every key that names a file (`main`, `icon`, `file`, `shader`) takes a path inside the package. It
is relative, written with `/` between folders, and never holds `..`, `.`, `\` or `:`. The file must
be in the package, or the install is refused.

### Ids and names

A plugin's id is lower-case words joined by dots, as in `acme.metrics`. A word holds letters,
digits and hyphens, begins with a letter and does not end with a hyphen. An id is 64 characters at
most. It cannot be `lume` or begin with `lume.`.

A name the plugin gives one of its own things (a command, a status item, a setting, a picture) has
the same form and is 31 characters at most: `refresh`, `show-all`. Within one kind of thing a name
may be given once.

### References

Where a key names something, it can be written three ways:

| Written as          | Names                                                                          |
| ------------------- | ------------------------------------------------------------------------------ |
| `refresh`           | One of the plugin's own things.                                                |
| `lume:zoom`         | One of the application's own places in a menu or a settings page.              |
| `lume.palette.open` | One of the application's commands, by its id.                                  |
| `acme.core:open`    | Something of another plugin's. That plugin must be among the `[dependencies]`. |

The other plugin is named by the id its manifest gives it, whatever id it is installed under.

## Activation events

`activation` lists the events that start the plugin's code.

| Event                   | Starts the code when                                                       |
| ----------------------- | -------------------------------------------------------------------------- |
| `onStartup`             | Lume has started and its first terminal has shown its first output.        |
| `onCommand:<name>`      | One of the plugin's commands is run. `<name>` must be one of its commands. |
| `onView:<name>`         | One of the plugin's status items, panels or pane views comes on screen.    |
| `onSessionEvent:<kind>` | A session reports something of this kind.                                  |
| `onService:<name>`      | Another plugin calls one of the plugin's services.                         |

The kinds of session event are `title`, `cwd`, `bell`, `command_started`, `command_finished`,
`program_status`, `foreground`, `activity`, `disconnected`, `state` and `exit`. A plugin is started
by a session event only if it holds the `workspace.read` permission.

`state` is a session's whole state at once: its title, folder, running command, the program in
front. It is reported for every session that was there before the plugins were read, such as a
terminal that came back with the window, and again where reports were missed. A plugin that must
know what already runs in the terminals lists `onSessionEvent:state`; the other kinds only tell of
what happens from then on.

A plugin is also started whenever one of its commands or services is called, or a row of one of its
panels is opened, whether or not the manifest lists the event. Listing events is how a plugin says
what else should start it. See [Performance](performance.md#choose-activation-events) for how to
choose.

## `[permissions]`

What the plugin may do beyond its own data. [Permissions](permissions.md) has the kinds and their
scopes.

```toml
[permissions]
"system.metrics" = true
"fs.read" = ["${workspace}/**"]
"process.run" = { scope = ["git"], prompt = "each" }

[permissions.optional]
"net" = ["api.example.com"]
```

## `[limits]`

| Key      | Type    | Default | What it is                                                    |
| -------- | ------- | ------- | ------------------------------------------------------------- |
| `memory` | integer | `64`    | The memory the plugin's code may use, in MiB: from 16 to 256. |

## `[dependencies]`

Other plugins this one uses, by the ids their manifests give them, each with the versions it works
with.

```toml
[dependencies]
"acme.core" = "^1.2"
"acme.extra" = { version = "1", optional = true }
```

| Form                                                      | Meaning                                                     |
| --------------------------------------------------------- | ----------------------------------------------------------- |
| `"<id>" = "<requirement>"`                                | Needed, at a version that meets the requirement.            |
| `"<id>" = { version = "<requirement>", optional = true }` | The same, as a table. `version` left out means any version. |

`optional = true` lets the plugin be used when the other is not installed. A plugin cannot depend
on itself. See [load order](install.md#load-order) for what a dependency changes.

## `[load]`

| Key      | Type            | What it is                                                      |
| -------- | --------------- | --------------------------------------------------------------- |
| `before` | list of strings | Ids of plugins this one loads before, where they are installed. |

## `[[contributes.commands]]`

A command: something the user runs from the palette, a key or a menu, carried out by the plugin's
code.

| Key        | Type    | Default           | What it is                                                     |
| ---------- | ------- | ----------------- | -------------------------------------------------------------- |
| `id`       | string  | needed            | The command's name. Its full id is `<plugin id>.<name>`.       |
| `title`    | string  | needed            | What it is called. One line, 64 characters at most.            |
| `category` | string  | the plugin's name | The heading the palette lists it under. 64 characters at most. |
| `palette`  | boolean | `true`            | Whether the palette offers it.                                 |

## `[[contributes.keybindings]]`

A key that runs a command. See [keybindings](settings.md#keybindings) for which keys a plugin may
bind.

| Key       | Type   | Default | What it is                                                                  |
| --------- | ------ | ------- | --------------------------------------------------------------------------- |
| `key`     | string | needed  | The keys, as in `ctrl+shift+y`, or several chords with spaces between them. |
| `macos`   | string | none    | The keys on a Mac, where they differ.                                       |
| `command` | string | needed  | The plugin's own command, one of the application's, or a dependency's.      |
| `args`    | any    | none    | Arguments given to the command.                                             |
| `when`    | string | none    | When the key holds: a [condition](placement.md#conditions).                 |

## `[[contributes.menus]]`

An item of a menu, or a submenu. See [Placement](placement.md#menus).

| Key        | Type    | Default             | What it is                                                      |
| ---------- | ------- | ------------------- | --------------------------------------------------------------- |
| `id`       | string  | needed              | The item's name.                                                |
| `location` | string  | needed              | The menu it is in. See [the menus](placement.md#menus).         |
| `section`  | string  | none                | The section of that menu it is in.                              |
| `parent`   | string  | none                | The submenu it is in, instead of the menu itself.               |
| `command`  | string  | none                | The command it runs. Needed unless `submenu = true`.            |
| `args`     | any     | none                | Arguments given to the command.                                 |
| `title`    | string  | the command's title | What it is called. Needed for a submenu. 64 characters at most. |
| `submenu`  | boolean | `false`             | Whether it opens a menu of its own. A submenu runs no command.  |
| `icon`     | string  | none                | A [picture](motion.md#pictures).                                |
| `when`     | string  | none                | A [condition](placement.md#conditions) it is shown under.       |
| `before`   | string  | none                | The neighbour it goes right before.                             |
| `after`    | string  | none                | The neighbour it goes right after.                              |
| `order`    | integer | none                | Where it goes when no neighbour is named. Lower comes first.    |

Give `section` or `parent`, not both. Give `before` or `after`, not both. That last rule holds for
every table with those keys.

## `[[contributes.sections]]`

A section of a menu: items a line sets apart, with a heading if it has a title.

| Key        | Type    | Default | What it is                             |
| ---------- | ------- | ------- | -------------------------------------- |
| `id`       | string  | needed  | The section's name.                    |
| `location` | string  | needed  | The menu it is in.                     |
| `title`    | string  | none    | Its heading. 64 characters at most.    |
| `before`   | string  | none    | The neighbour it goes right before.    |
| `after`    | string  | none    | The neighbour it goes right after.     |
| `order`    | integer | none    | Where it goes when no neighbour named. |

## `[[contributes.status]]`

A status item: a small piece of information, or a button, in a bar. See
[status items](placement.md#status-items).

| Key            | Type    | Default | What it is                                                                                                                                |
| -------------- | ------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `id`           | string  | needed  | The item's name.                                                                                                                          |
| `slot`         | string  | none    | Where it goes: `topbar.left`, `topbar.center`, `topbar.right`, `statusbar.left`, `statusbar.right` or `pane.header`.                      |
| `slot-setting` | string  | none    | One of the plugin's settings whose value is the place it goes in.                                                                         |
| `kind`         | string  | needed  | `text`, `icon`, `meter`, `sparkline`, `badge` or `segmented`.                                                                             |
| `choices`      | array   | none    | The parts of a `segmented` item, from two to four, each `{ value, label, icon? }`. See [a segmented item](placement.md#a-segmented-item). |
| `label`        | string  | none    | What the item stands for. 64 characters at most.                                                                                          |
| `icon`         | string  | none    | A [picture](motion.md#pictures).                                                                                                          |
| `tooltip`      | string  | none    | What the pointer is told. 300 characters at most.                                                                                         |
| `command`      | string  | none    | A command the item runs when clicked, which makes it a button.                                                                            |
| `menu`         | string  | none    | A submenu of the plugin's own, in `menu.status`, the item opens when clicked.                                                             |
| `when`         | string  | none    | A [condition](placement.md#conditions) it is shown under.                                                                                 |
| `shown-when`   | string  | none    | One of the plugin's boolean settings that must be on for the item to be shown.                                                            |
| `before`       | string  | none    | The neighbour it goes right before.                                                                                                       |
| `after`        | string  | none    | The neighbour it goes right after.                                                                                                        |
| `order`        | integer | none    | Where it goes when no neighbour is named.                                                                                                 |

Give `slot` or `slot-setting`: one of them is needed, and not both. Give `command` or `menu`, not
both.

## `[[contributes.settings]]`

One of the plugin's settings. See [Settings](settings.md).

| Key          | Type           | Default       | What it is                                                                |
| ------------ | -------------- | ------------- | ------------------------------------------------------------------------- |
| `key`        | string         | needed        | The setting's name inside the plugin's table of the configuration.        |
| `type`       | string         | needed        | `boolean`, `integer`, `number`, `string` or `table`.                      |
| `default`    | value          | needed        | Its value until the user sets one. It must be of the type.                |
| `minimum`    | number         | none          | The least it may be. Only for `integer` and `number`.                     |
| `maximum`    | number         | none          | The most it may be. Only for `integer` and `number`.                      |
| `choices`    | list of tables | none          | The values it may have, each `{ value = ..., label = "..." }`.            |
| `label`      | string         | none          | What the settings window calls it. Without one it is only in the file.    |
| `help`       | string         | none          | A line under the label. 300 characters at most.                           |
| `control`    | string         | from the type | `switch`, `select`, `segmented`, `text` or `number`.                      |
| `page`       | string         | `self`        | `self` for the plugin's own page, or `general`, `appearance`, `terminal`. |
| `group`      | string         | none          | The group of that page it is in.                                          |
| `unit`       | string         | none          | What the value is counted in, as `px` or `s`.                             |
| `step`       | number         | `1`           | How far one step of a number control goes.                                |
| `shown-when` | string         | none          | Another of the plugin's boolean settings that must be on to show this.    |
| `before`     | string         | none          | Another of the plugin's settings it goes right before.                    |
| `after`      | string         | none          | Another of the plugin's settings it goes right after.                     |
| `order`      | integer        | none          | Where it goes when no neighbour is named.                                 |

A `table` setting is written in the configuration file only: of the keys above it takes `key`,
`type`, `default`, `label` and `help`. See [tables](settings.md#tables).

## `[[contributes.setting-groups]]`

A group of settings on a page, with a heading.

| Key      | Type    | Default | What it is                                        |
| -------- | ------- | ------- | ------------------------------------------------- |
| `id`     | string  | needed  | The group's name.                                 |
| `title`  | string  | needed  | Its heading. 64 characters at most.               |
| `page`   | string  | `self`  | `self`, or `general`, `appearance` or `terminal`. |
| `before` | string  | none    | The group it goes right before.                   |
| `after`  | string  | none    | The group it goes right after.                    |
| `order`  | integer | none    | Where it goes when no neighbour is named.         |

## `[[contributes.themes]]`

A theme file in the package. See [themes](settings.md#themes).

| Key    | Type   | What it is                           |
| ------ | ------ | ------------------------------------ |
| `id`   | string | The theme's name.                    |
| `file` | string | A `.toml` theme file in the package. |

## `[[contributes.panels]]`

A panel in a dock area of the window: a `list`, drawn by Lume from rows the plugin gives (see
[panels](placement.md#panels)), or a `frame`, which shows a page of the plugin's own (see
[views](views.md)).

| Key      | Type    | Default | What it is                                                                                             |
| -------- | ------- | ------- | ------------------------------------------------------------------------------------------------------ |
| `id`     | string  | needed  | The panel's name.                                                                                      |
| `title`  | string  | needed  | What it is called. 64 characters at most.                                                              |
| `kind`   | string  | needed  | How it is drawn: `list` or `frame`.                                                                    |
| `entry`  | string  | none    | The page a `frame` panel shows: an `.html` file in the package. Needed for `frame`, refused otherwise. |
| `area`   | string  | needed  | The dock area it opens in the first time: `left`, `right` or `bottom`.                                 |
| `icon`   | string  | none    | A [picture](motion.md#pictures).                                                                       |
| `before` | string  | none    | The panel it goes right before.                                                                        |
| `after`  | string  | none    | The panel it goes right after.                                                                         |
| `order`  | integer | none    | Where it goes when no neighbour is named.                                                              |

A `list` panel needs the plugin to have code (`main`), which gives its rows. A `frame` panel does
not.

## `[[contributes.pane-views]]`

A page of the plugin's own that is shown in a pane, where a terminal would be. See
[views](views.md).

| Key       | Type    | Default | What it is                                                                   |
| --------- | ------- | ------- | ---------------------------------------------------------------------------- |
| `id`      | string  | needed  | The view's name. A pane view and a panel of one plugin may not share a name. |
| `title`   | string  | needed  | What its pane is called until the plugin titles it. 64 characters at most.   |
| `entry`   | string  | needed  | The page: an `.html` file in the package.                                    |
| `icon`    | string  | none    | A [picture](motion.md#pictures), which its pane is known by.                 |
| `persist` | boolean | `true`  | Whether what the view asks to have kept is saved with the workspace.         |

## `[[contributes.openers]]`

Files and addresses one of the plugin's pane views opens. When the user opens a file or an address
that an opener matches, the view is offered for it; where it is the only thing that opens it, the
view opens at once. The view starts with what was opened as its [state](views.md#state).

| Key        | Type            | Default | What it is                                                                                                                                 |
| ---------- | --------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `view`     | string          | needed  | One of the plugin's pane views.                                                                                                            |
| `title`    | string          | none    | What the choice is called where the user is asked; the view's title if left out.                                                           |
| `patterns` | list of strings | none    | File names it opens, such as `*.diff`. `*` and `?` stand for any characters within a name; upper and lower case are the same.              |
| `schemes`  | list of strings | none    | Kinds of address it opens, such as `acme-review` for `acme-review:…`. The schemes of the web, of files and of Lume itself cannot be taken. |

An opener names at least one pattern or scheme.

## `[[contributes.cli]]`

A verb for the command line. `lume <verb> [arguments]` runs one of the plugin's commands in the
running application, prints what it gives back and ends with its status.

```toml
[[contributes.commands]]
id = "show"
title = "Show the figures"
palette = false

[[contributes.cli]]
verb = "metrics show"
command = "show"
summary = "Print the figures once"
usage = "lume metrics show [--json]"
```

| Key       | Type   | Default | What it is                                                                            |
| --------- | ------ | ------- | ------------------------------------------------------------------------------------- |
| `verb`    | string | needed  | One to three words with a space between them: lower-case letters, digits and hyphens. |
| `command` | string | needed  | One of the plugin's own commands, by its name.                                        |
| `summary` | string | needed  | What it does, in a line. `lume ctl verbs` lists it. 300 characters at most.           |
| `usage`   | string | none    | How it is written, for the plugin's own help. 300 characters at most.                 |

- The first word cannot be one of lume's own: `help`, `ctl`, `host`, `plugin`, `config`, `keymap`,
  `theme`, `shell-integration`.
- Where the words on the command line begin with more than one verb, the one with the most words
  is meant: with `metrics` and `metrics show` both there, `lume metrics show x` runs the second.
- A verb belongs to the first plugin that adds it, in the order plugins load in. Another plugin
  that adds the same verb is told so on its page of the add-ons.
- The application has to be running: it is the application that runs the plugin's code. The
  plugin is started for the verb if it is not running.

### The command of a verb

The command is called with an object:

| Key     | Type            | What it is                                                                                                    |
| ------- | --------------- | ------------------------------------------------------------------------------------------------------------- |
| `argv`  | list of strings | Everything written after the verb, as written. The plugin reads its own options from it, `--help` among them. |
| `cwd`   | string          | The directory `lume` was run in.                                                                              |
| `stdin` | string          | What was piped in, if anything was: text, 512 KiB at most.                                                    |

It gives back an object, or nothing:

| Key    | Type    | What it is                                                                        |
| ------ | ------- | --------------------------------------------------------------------------------- |
| `out`  | string  | Printed on standard output, as it is: end lines yourself. 256 KiB at most a step. |
| `err`  | string  | Printed on standard error. 256 KiB at most a step.                                |
| `next` | any     | If given, the command is called again, with `{ next }`. 64 KiB at most as JSON.   |
| `code` | integer | The status `lume` ends with, 0 to 255, when there is no `next`. 0 if left out.    |

A verb that takes a while runs in steps. Each call gives back what there is to print so far and a
`next` to be called again with; the call without a `next` ends the verb. What the plugin needs to
find the run again (an id it made up) travels in `next`.

```js
const runs = new Map();
let started = 0;

lume.commands.register("show", async (args) => {
  if (args.cancel) {
    runs.get(args.next)?.stop();
    runs.delete(args.next);
    return {};
  }
  if (args.next === undefined) {
    if (args.argv.includes("--help")) return { out: "usage: lume metrics show [--json]\n" };
    const id = ++started;
    runs.set(id, startReading(args.argv));
    return { err: "reading...\n", next: id };
  }
  const run = runs.get(args.next);
  // Wait for more, but not for long: give back within about 20 seconds.
  const lines = await run.more(15_000);
  if (!run.done) return { out: lines, next: args.next };
  runs.delete(args.next);
  return { out: lines, code: run.failed ? 1 : 0 };
});
```

- Each call has the time any command has, 30 seconds, not counting time the user is being asked
  something. Give back within about 20 seconds, with nothing to print if there is nothing, and be
  called again. A verb may go on for as long as it likes this way.
- If the user interrupts `lume`, or it goes away, the command is called once more with
  `{ next, cancel: true }`. Stop what the run started; what this call gives back is not printed.
- A call that throws, runs out of time or gives back something else ends the verb with status 1
  and a line on standard error.
- `lume` reads its standard input to the end before it asks for the verb, when that is not a
  terminal. A verb started with a pipe that stays open waits for it.

## `[[contributes.services]]`

A function other plugins may call. See [services](permissions.md#services-between-plugins).

| Key           | Type   | Default | What it is                        |
| ------------- | ------ | ------- | --------------------------------- |
| `name`        | string | needed  | The service's name.               |
| `description` | string | none    | One line, 300 characters at most. |

## `[[contributes.images]]`

A picture in the package, by the name the plugin's other things and its code give it. See
[pictures](motion.md#pictures).

| Key    | Type    | Default | What it is                                                                 |
| ------ | ------- | ------- | -------------------------------------------------------------------------- |
| `id`   | string  | needed  | The picture's name.                                                        |
| `file` | string  | needed  | An `.svg`, `.png`, `.jpg`, `.jpeg`, `.webp` or `.gif` file in the package. |
| `tint` | boolean | `false` | Whether it is drawn in the colour of what it stands beside.                |

## `[[contributes.motions]]`

An effect for one of the window's own places. See [motions](motion.md#motions).

| Key         | Type           | Default    | What it is                                                                |
| ----------- | -------------- | ---------- | ------------------------------------------------------------------------- |
| `id`        | string         | needed     | The effect's name.                                                        |
| `title`     | string         | needed     | What the settings call it. 64 characters at most.                         |
| `slot`      | string         | needed     | The place it plays in.                                                    |
| `keyframes` | list of tables | none       | From 2 to 8 keyframes. Needed for every place but `tab.highlight`.        |
| `duration`  | integer        | none       | Milliseconds, from 1 to 1000. Needed for every place but `tab.highlight`. |
| `easing`    | string         | `standard` | Its curve.                                                                |
| `delay`     | integer        | `0`        | Milliseconds it waits before it starts: 500 at most.                      |
| `highlight` | table          | none       | For `tab.highlight` only, and needed there.                               |

## `[[contributes.cursor-effects]]`

An effect the cursor plays, drawn by a shader in the package. See
[cursor effects](motion.md#cursor-effects).

| Key        | Type    | Default | What it is                                                           |
| ---------- | ------- | ------- | -------------------------------------------------------------------- |
| `id`       | string  | needed  | The effect's name.                                                   |
| `title`    | string  | needed  | What the settings call it. 64 characters at most.                    |
| `kind`     | string  | needed  | `trail` or `pulse`.                                                  |
| `shader`   | string  | needed  | A `.glsl` file in the package.                                       |
| `duration` | integer | `400`   | How long it lasts, in milliseconds: from 1 to 1500.                  |
| `reach`    | integer | `48`    | How far from the cursor's cells it may draw, in pixels: 256 at most. |

## A larger example

Every key here passes the manifest's checks.

```toml
schema = 1
id = "acme.metrics"
name = "Metrics"
version = "0.1.0"
api = "1"
main = "dist/host.js"
activation = ["onView:cpu", "onCommand:refresh", "onSessionEvent:command_finished"]

[permissions]
"system.metrics" = true
"workspace.read" = true

[limits]
memory = 32

[[contributes.commands]]
id = "refresh"
title = "Refresh meters"

[[contributes.keybindings]]
key = "ctrl+shift+y"
command = "refresh"

[[contributes.sections]]
id = "meters"
location = "menu.app.view"
title = "Meters"
after = "lume:zoom"

[[contributes.menus]]
id = "refresh-item"
location = "menu.app.view"
section = "meters"
command = "refresh"

[[contributes.menus]]
id = "cpu-menu"
location = "menu.status"
title = "CPU"
submenu = true

[[contributes.menus]]
id = "cpu-refresh"
location = "menu.status"
parent = "cpu-menu"
command = "refresh"

[[contributes.status]]
id = "cpu"
slot-setting = "place"
kind = "meter"
label = "CPU"
menu = "cpu-menu"
shown-when = "show-cpu"

[[contributes.setting-groups]]
id = "sampling"
title = "Sampling"

[[contributes.settings]]
key = "show-cpu"
type = "boolean"
default = true
label = "CPU"

[[contributes.settings]]
key = "place"
type = "string"
default = "statusbar.right"
label = "Show in"
control = "segmented"
choices = [
  { value = "statusbar.right", label = "Status bar" },
  { value = "topbar.right", label = "Top bar" },
]

[[contributes.settings]]
key = "rate"
type = "integer"
default = 2
label = "Refresh every"
choices = [{ value = 1, label = "1 s" }, { value = 2, label = "2 s" }]
group = "sampling"
shown-when = "show-cpu"

[[contributes.panels]]
id = "processes"
title = "Processes"
kind = "list"
area = "right"

[[contributes.services]]
name = "latest"
```
