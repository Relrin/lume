# Placement

A plugin adds to places the window already has: its menus, its bars, its dock areas. This page
says what those places are, how a plugin's things are ordered among their neighbours, and what
happens to something that cannot be put where its manifest asks.

Everything here comes from the manifest. Lume builds the menus, bars and panels of every installed
plugin without running any plugin's code.

## Menus

`location` names one of these menus:

| Menu            | What it is                                                                 |
| --------------- | -------------------------------------------------------------------------- |
| `menu.app`      | The application's menu bar. Its items are the menus Lume, View and Help.   |
| `menu.app.lume` | The Lume menu.                                                             |
| `menu.app.view` | The View menu.                                                             |
| `menu.app.help` | The Help menu.                                                             |
| `menu.new-tab`  | The "New tab with" menu.                                                   |
| `menu.tab`      | A tab's menu.                                                              |
| `menu.pane`     | A pane's menu.                                                             |
| `menu.status`   | The menus that status items open. It has nothing of the application's own. |

A menu holds sections, and sections hold items. An item can also sit at the top of a menu, outside
every section, or inside a submenu.

### Items

```toml
[[contributes.commands]]
id = "refresh"
title = "Refresh meters"

[[contributes.menus]]
id = "refresh-item"
location = "menu.app.view"
section = "lume:zoom"
after = "lume:zoom-reset"
command = "refresh"
```

Where an item goes:

- With `parent`, inside that submenu.
- With `section`, in that section of the menu.
- With neither, at the top of the menu.

What an item runs:

- One of the plugin's own commands, by its name. The item is called by the command's title unless
  it has a `title` of its own.
- One of a dependency's commands, as `acme.core:open`.
- One of the application's commands, by its id. The item is always called what the command is
  called, whatever `title` says. Only the application's commands that need no permission can be
  run this way: see [the application's commands](permissions.md#the-applications-commands).

### Sections

A section is a run of items a line sets apart. With a `title` it has a heading.

```toml
[[contributes.sections]]
id = "meters"
location = "menu.app.view"
title = "Meters"
after = "lume:zoom"

[[contributes.menus]]
id = "view-refresh"
location = "menu.app.view"
section = "meters"
command = "refresh"
```

A section goes at the top of its menu, among the menu's other sections. The menu bar `menu.app`
takes no sections.

### Submenus

An item with `submenu = true` opens a menu of its own. It needs a `title` and runs no command.
Other items name it as their `parent`.

```toml
[[contributes.menus]]
id = "tools"
location = "menu.app"
title = "Tools"
submenu = true
after = "lume:view"

[[contributes.menus]]
id = "tools-refresh"
location = "menu.app"
parent = "tools"
command = "refresh"
```

This adds a Tools menu to the menu bar, after View, with one item in it.

- The menu bar `menu.app` holds only menus. An item placed at its top must be a submenu.
- An item and the submenu it is in have the same `location`.
- Submenus go three deep at most.
- `parent` can name one of the application's submenus that is a menu of its own. The item then goes
  at the top of that menu: `parent = "lume:view"` in `menu.app` puts it in the View menu, and
  `parent = "lume:new-tab-with"` in `menu.app.lume` puts it in the "New tab with" menu.

### The application's own nodes

Each of the application's sections, items and submenus has a key. A manifest writes it as
`lume:<key>` to put something in a section (`section`), in a submenu (`parent`), or next to a
neighbour (`before`, `after`).

| Menu            | Section       | Its items, in order                                                       |
| --------------- | ------------- | ------------------------------------------------------------------------- |
| `menu.app`      | (none)        | `lume`, `view`, `help`: the three menus                                   |
| `menu.app.lume` | `new`         | `new-window`, `new-tab`, `new-tab-with` (a submenu), `groups` (a submenu) |
| `menu.app.lume` | `workspaces`  | `new-workspace`, `open-workspace`                                         |
| `menu.app.lume` | `preferences` | `settings`                                                                |
| `menu.app.lume` | `sessions`    | `detached`                                                                |
| `menu.app.lume` | `leave`       | `quit`, `quit-end-sessions`                                               |
| `menu.app.view` | `splits`      | `split-right`, `split-down`                                               |
| `menu.app.view` | `zoom`        | `zoom-in`, `zoom-out`, `zoom-reset`                                       |
| `menu.app.view` | `panels`      | `panel-list`                                                              |
| `menu.app.help` | `about`       | `about-lume`, `whats-new`                                                 |
| `menu.app.help` | `support`     | `commands`, `report-issue`, `show-logs`                                   |
| `menu.app.help` | `host`        | `restart-host`                                                            |
| `menu.new-tab`  | `default`     | `default-terminal`                                                        |
| `menu.new-tab`  | `profiles`    | `profile-list`                                                            |
| `menu.new-tab`  | `more`        | `more-profiles`                                                           |
| `menu.tab`      | `name`        | `rename`, `color`                                                         |
| `menu.tab`      | `group`       | `new-group`, `to-group`, `remove-from-group`                              |
| `menu.tab`      | `position`    | `new-tab`, `move-left`, `move-right`, `move-into`                         |
| `menu.tab`      | `closing`     | `mute`, `lock`, `close-left`, `close-right`, `close`                      |
| `menu.pane`     | `name`        | `rename`                                                                  |
| `menu.pane`     | `show`        | `show-in-new-tab`, `show-in-new-window`                                   |

The `groups` submenu takes no items of plugins.

A key names one thing in one menu. `lume:new-tab` in `menu.tab` is not the `lume:new-tab` of
`menu.app.lume`.

## Order: `before`, `after` and `order`

Things are ordered inside one container at a time: the top of a menu, one section, one submenu, the
window's bars, one dock area, one group of settings.

- `before = "<neighbour>"` puts the thing right before that neighbour.
- `after = "<neighbour>"` puts it right after.
- `order = <number>` places it where no neighbour is named. Lower comes first.

Give `before` or `after`, never both.

A neighbour is written as a [reference](manifest.md#references): one of the plugin's own things by
its name, one of the application's as `lume:<key>`, or a dependency's as `<plugin id>:<name>`.

The rules, in full:

1. A neighbour that is not in the same container is set aside, with a warning, and the thing is
   placed by its `order`.
2. Things with no neighbour named come first by `order`. Where two have the same order, the
   application's own come before any plugin's, then plugins go in the order they load in, then each
   plugin's things go in the order its manifest declares them.
3. Each of those is preceded by the things placed `before` it and followed by the things placed
   `after` it, each of those with its own in turn.
4. Things placed against each other in a circle lose their neighbours, with a warning, and are
   placed by their orders.

### Order numbers

The application's own things have orders spaced apart, so a number can fall between two of them:

| What                                            | Orders             |
| ----------------------------------------------- | ------------------ |
| The application's sections at the top of a menu | 100, 200, 300, ... |
| The application's items inside a section        | 10, 20, 30, ...    |
| The three menus of the menu bar                 | 10, 20, 30         |
| A plugin's thing that gives no `order`          | 10 000             |

So a section with `order = 150` in `menu.app.view` goes between `splits` and `zoom`, and an item
with `order = 15` in the `zoom` section goes between `zoom-in` and `zoom-out`. A plugin's thing
that says nothing goes after the application's own.

### Other plugins' things

To place something next to another plugin's thing, or inside its section or submenu, name that
plugin in `[dependencies]` and write the reference with the id its manifest gives it:

```toml
[dependencies]
"acme.core" = { version = "^1", optional = true }

[[contributes.menus]]
id = "extra"
location = "menu.app.view"
section = "acme.core:tools"
command = "refresh"
```

A dependency loads first, so its things are there to be placed against. If the user has bound the
dependency to another install, such as a fork, the reference follows the binding. See
[binding dependencies](install.md#binding-dependencies).

## Status items

A status item is a small piece of information, or a button, in a bar.

```toml
[[contributes.status]]
id = "branch"
slot = "statusbar.left"
kind = "text"
label = "Branch"
tooltip = "The branch of the workspace"
```

### Places

| Slot              | Where                                                   |
| ----------------- | ------------------------------------------------------- |
| `topbar.left`     | The left of the top bar.                                |
| `topbar.center`   | The middle of the top bar.                              |
| `topbar.right`    | The right of the top bar.                               |
| `statusbar.left`  | The left end of the status bar along the window's foot. |
| `statusbar.right` | The right end of the status bar.                        |
| `pane.header`     | The header of every pane.                               |

An item in one of the bars is shown once for the window. An item in `pane.header` is shown in each
pane's header, and its value is set for one pane at a time: `lume.status.set(id, value, pane)`.

The items of all five places of the bars are ordered together, and those of the panes' headers
apart. `before` and `after` name another status item.

### Kinds

| Kind        | Shows                                   |
| ----------- | --------------------------------------- |
| `text`      | Words.                                  |
| `icon`      | A picture.                              |
| `meter`     | How full something is, from 0 to 1.     |
| `sparkline` | A small line through the latest values. |
| `badge`     | A count or a word.                      |
| `segmented` | A few choices, one of them chosen.      |

Until the plugin's code says more, an item shows what its manifest gives it: its `label`, its
`icon`, its `tooltip`.

### A segmented item

A `segmented` item is a small switch: from two to four parts side by side, one of them the
chosen one. It suits a pane's header, where it says which of a few ways the pane is shown.

```toml
[[contributes.status]]
id = "view"
slot = "pane.header"
kind = "segmented"
label = "Pane view"
command = "show-as"
choices = [
  { value = "terminal", label = "Terminal", icon = "terminal" },
  { value = "ui", label = "UI", icon = "window" },
]
```

- Each choice has a `value` (lower-case letters, digits and hyphens), a `label`, and may have
  an `icon`. In a narrow pane a choice with an icon is drawn as the icon alone.
- The item needs a `command`, and takes no `menu`. Pressing a part that is not the chosen
  one runs the command with `{ value }`, and in a pane's header with `{ pane, value }`.
- The code says which part is chosen: `lume.status.set(id, { selected: "ui" }, pane)`. The
  window does not change it by itself when a part is pressed: the command does what was
  asked, and then says what is chosen now.

### Values

The code shows something in an item with `lume.status.set`:

```js
await lume.status.set("cpu", {
  value: 0.42,
  text: "42%",
  color: "color.accent",
  card: [
    { label: "Now", value: "42%", points: [0.31, 0.38, 0.42] },
    { label: "Highest lately", value: "77%" },
  ],
});
```

Fields left out keep what the item showed before. The fields are in the
[API reference](api.md#statusvalue): `text`, `value`, `points`, `label`, `icon`, `tone`, `color`,
`tooltip`, `hidden` and `card`.

- `color` is one of the theme's colours by its token's name, such as `color.accent` or
  `color.tab.purple`. Anything else is refused as `invalid`.
- `card` is the rows of a card shown when the pointer rests on the item. Each row has a `label` and
  a `value`, and may have `points` and a `tone`.
- `hidden: true` takes the item out of sight for now.

`lume.status.clear(id)` takes back what the code showed, so the item shows what its manifest says.

Neither call needs a permission. Both fail as `not_found` for an id the manifest does not declare.

### Buttons and menus

`command` makes the item a button that runs a command when clicked: one of the plugin's own, a
dependency's, or one of the application's that needs no permission.

`menu` makes it open a menu when clicked. The menu is one of the plugin's own submenus, and that
submenu must be in `menu.status`:

```toml
[[contributes.menus]]
id = "cpu-menu"
location = "menu.status"
title = "Processor"
submenu = true

[[contributes.menus]]
id = "cpu-refresh"
location = "menu.status"
parent = "cpu-menu"
command = "refresh"

[[contributes.status]]
id = "cpu"
slot = "statusbar.right"
kind = "meter"
label = "CPU"
menu = "cpu-menu"
```

Give `command` or `menu`, not both.

### `shown-when`: a switch for the item

`shown-when` names one of the plugin's boolean settings. The item is shown only while that setting
is on.

```toml
[[contributes.status]]
id = "cpu"
slot = "statusbar.right"
kind = "meter"
label = "CPU"
shown-when = "show-cpu"

[[contributes.settings]]
key = "show-cpu"
type = "boolean"
default = true
label = "CPU"
```

The window reads the setting itself. An item the user turns off goes at once, and one they turn on
comes back, without the plugin's code. An item that comes back counts as coming on screen, so it
starts a plugin whose manifest lists `onView:cpu`.

### `slot-setting`: a place the user chooses

`slot-setting` names one of the plugin's settings whose value is the place the item goes in. Use it
in place of `slot`.

```toml
[[contributes.status]]
id = "cpu"
slot-setting = "place"
kind = "meter"
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
```

The setting must be a `string` setting with `choices`, and every choice must be one of the five
places of the bars. `pane.header` cannot be chosen this way.

The window reads the setting itself, so the item moves as soon as the setting changes, without the
plugin's code. Because the bars' items are ordered together, an item that moves keeps its place
among the others where it lands.

## Panels

A panel sits in a dock area of the window. Lume draws it from rows the plugin gives.

```toml
[[contributes.panels]]
id = "files"
title = "Files"
kind = "list"
area = "right"
```

- `kind` is `list`: a list of rows, which can hold rows of their own.
- `area` is the dock area the panel opens in the first time: `left`, `right` or `bottom`.
- `before`, `after` and `order` place it among the other plugins' panels of its area.

A panel needs code. The code gives its rows:

```js
await lume.panels.set("files", [
  {
    id: "src",
    label: "src",
    badge: "3",
    expanded: true,
    children: [
      { id: "main", label: "main.c", detail: "changed" },
      { id: "util", label: "util.c" },
    ],
  },
  {
    id: "readme",
    label: "README",
    actions: [{ title: "Open", command: "open" }],
  },
]);
```

- `lume.panels.set(id, rows)` shows rows in place of those the panel showed.
- `lume.panels.clear(id)` takes them away.
- A row has an `id` that names it among its siblings, a `label`, and may have `detail`, `badge`,
  `icon`, `tone`, `children`, `expanded`, `lazy` and `actions`. See
  [`PanelRow`](api.md#panelrow).
- Each action runs one of the plugin's own commands. The handler is given
  `{ panel, row, path }` (the panel's name, the row's id and the ids from the top down to the row)
  together with the action's own `args`. The first action is what choosing the row does.

Limits: a panel holds 5000 rows at most, eight levels deep. A row's words are one line of 200
characters at most. Rows that break a limit are refused as `invalid`.

A tree too large to build whole gives its rows as they are opened. See
[lazy rows](performance.md#lazy-rows).

List `onView:<panel>` among the activation events to start the code when the panel comes on screen.

## Conditions

`when` on a menu item, a status item or a keybinding says when it is shown, or when the key holds.
A condition is names joined by `&&`, each perhaps after `!`. All must hold.

```toml
[[contributes.menus]]
id = "stop"
location = "menu.pane"
command = "stop"
when = "plugin:busy"
```

The plugin's code sets its own conditions:

```js
await lume.context.set("busy", true);
```

| Written in the manifest        | Holds while                                |
| ------------------------------ | ------------------------------------------ |
| `plugin:<key>`                 | The plugin itself has set `<key>` to true. |
| `!plugin:<key>`                | It has not.                                |
| `plugin:<dependency id>:<key>` | That dependency has set `<key>` to true.   |

A key is a name like any other the plugin gives: lower-case words, 31 characters at most. A
condition nobody has set does not hold.

A keybinding's `when` can also name the application's own contexts, as the configuration's
keybindings do: `terminal`, `alt-screen`, `floating`, `palette`, `search`, `settings`, and
`panel:<panel id>`.

## What cannot be placed

Lume never refuses a whole plugin for a placement that does not work out. It leaves the one thing
out, or places it by its order, and says so in a warning.

| What is wrong                                                          | What happens                        |
| ---------------------------------------------------------------------- | ----------------------------------- |
| `before` or `after` names something that is not in the same container  | Placed by its `order`.              |
| Things are placed against each other in a circle                       | Placed by their `order`s.           |
| An item's `section` is not in its menu                                 | Placed at the top of the menu.      |
| An item's `parent` submenu is not in its menu                          | Left out.                           |
| An item's `parent` is a submenu of the application's that takes none   | Left out.                           |
| An item that is not a submenu at the top of `menu.app`                 | Left out.                           |
| A section in `menu.app`                                                | Left out.                           |
| A submenu more than three deep, or submenus that hold each other       | Left out.                           |
| An item runs one of the application's commands plugins may not run     | Left out.                           |
| An item runs a command of the application's that does not exist        | Left out.                           |
| An item runs a command of a dependency that is not installed           | Left out.                           |
| A status item's `command` is one plugins may not run                   | The item is shown and runs nothing. |
| A key runs a command plugins may not run, or one that is not installed | The key is not bound.               |
| A picture cannot be read, or the pictures are too large together       | The picture is left out.            |

The warnings are shown as `note:` lines under the plugin in `lume plugin list`, and with the plugin
under **Add-ons** in the settings. Whether plugins may run one of the application's commands is
known only to the running application, so those warnings are shown in the settings alone.
