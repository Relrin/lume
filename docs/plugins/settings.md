# Settings, themes and keybindings

This page covers what a plugin adds to the configuration: settings of its own, themes, and keys
that run commands.

## Settings

A setting is declared in the manifest. Lume then knows its type, its default and how to show it,
without the plugin's code.

```toml
[[contributes.settings]]
key = "rate"
type = "integer"
default = 2
minimum = 1
maximum = 60
label = "Refresh every"
help = "How often the meters are read"
unit = "s"
```

### Types

| `type`    | Holds             | `default` is          |
| --------- | ----------------- | --------------------- |
| `boolean` | `true` or `false` | a boolean             |
| `integer` | a whole number    | an integer            |
| `number`  | any number        | an integer or a float |
| `string`  | text              | a string              |

- `minimum` and `maximum` are for `integer` and `number` only. The default must lie within them.
- `choices` lists the values a setting may have, each with the label the settings window shows:
  `{ value = 2, label = "2 s" }`. Every value must be of the setting's type, and the default must
  be one of them.

### Controls

`control` says what the settings window shows the setting with. Left out, it follows from the type.

| `control`   | Fits                       | Used when `control` is left out for |
| ----------- | -------------------------- | ----------------------------------- |
| `switch`    | a `boolean`                | a `boolean`                         |
| `select`    | any setting with `choices` | any setting with `choices`          |
| `segmented` | any setting with `choices` | never                               |
| `number`    | an `integer` or a `number` | an `integer` or a `number`          |
| `text`      | a `string`                 | a `string`                          |

A number control steps by `step`, which is 1 unless given. `unit` is what the value is counted in,
such as `px` or `s`.

### Shown or only in the file

A setting with a `label` is shown in the settings window. A setting without one is only in the
configuration file: it is checked and has its default, but the window does not show it.

`shown-when` names another of the plugin's boolean settings. The row is shown only while that
setting is on:

```toml
[[contributes.settings]]
key = "show-cpu"
type = "boolean"
default = true
label = "CPU"

[[contributes.settings]]
key = "cpu-colour"
type = "string"
default = "accent"
label = "Colour"
choices = [{ value = "accent", label = "Accent" }, { value = "purple", label = "Purple" }]
shown-when = "show-cpu"
```

### The plugin's own page

`page` says which page of the settings window a setting is on. Left out, or `self`, it is the
plugin's own page.

A plugin with at least one labelled setting on its own page gets a page under the heading
**Add-ons**. The page is called by the plugin's name and shows its description, its version and its
`icon`.

Settings on the page are in groups. A group has a heading:

```toml
[[contributes.setting-groups]]
id = "meters"
title = "Status bar meters"

[[contributes.setting-groups]]
id = "sampling"
title = "Sampling"

[[contributes.settings]]
key = "cpu"
type = "boolean"
default = true
label = "CPU"
group = "meters"

[[contributes.settings]]
key = "rate"
type = "integer"
default = 2
label = "Refresh every"
choices = [{ value = 1, label = "1 s" }, { value = 2, label = "2 s" }, { value = 5, label = "5 s" }]
group = "sampling"
```

- A setting with no `group` goes under the heading **Settings**, which comes first on the page.
- Groups are placed with `before`, `after` and `order`, like everything else. See
  [order](placement.md#order-before-after-and-order).
- Settings inside a group are placed the same way. `before` and `after` name another of the
  plugin's settings.

### Rows on the application's pages

A setting can sit on one of the application's own pages instead. Give the page's id, and the group
of that page it goes in as `lume:<group>`:

```toml
[[contributes.settings]]
key = "cursor-glow"
type = "boolean"
default = false
label = "Glow"
page = "terminal"
group = "lume:cursor"
```

| Page         | Its groups                                                                         |
| ------------ | ---------------------------------------------------------------------------------- |
| `general`    | `lume:startup`, `lume:windows`, `lume:attention`, `lume:closing`, `lume:sessions`  |
| `appearance` | `lume:theme`, `lume:type`, `lume:panes`, `lume:tabs`, `lume:background`            |
| `terminal`   | `lume:new-terminals`, `lume:cursor`, `lume:history`, `lume:buffer`, `lume:closing` |

A group's key is its heading in lower case, with hyphens for spaces.

- A setting on one of the application's pages must name its `group`.
- Its row goes after the application's own rows of that group, unless its `order` puts it among
  them.
- The value is still the plugin's: it lives in the plugin's table of the configuration, not among
  the application's settings.

### Where the values live

The user's values are in the configuration file, `config.toml`, in a table named after the id the
plugin is installed under:

```toml
[plugins."acme.metrics"]
cpu = true
rate = 5
```

The settings window writes there, and the user can edit the file by hand. Lume checks the table
against the manifest: a key the plugin does not declare, or a value of the wrong type, outside the
bounds or not among the choices, is reported to the user as a problem in the configuration.

A setting the user has not set has its default.

### Reading settings in code

```js
const rate = await lume.settings.get("rate");
const all = await lume.settings.get();
```

`lume.settings.get(key)` gives one setting. With no key it gives all of them as one object. Neither
needs a permission.

`lume.settings.onChange(listener)` hears every change. The listener is given all of the plugin's
settings as one object, not only the one that changed:

```js
function apply(settings) {
  // Start or stop what the settings ask for.
}

lume.settings.onChange(apply);
lume.settings
  .get()
  .then(apply)
  .catch((error) => lume.log.error(error));
```

`onChange` is not called for the values the plugin starts with. Read them once with `get`, as
above.

Some settings need no code at all. A status item's `shown-when` and `slot-setting` are read by the
window itself. See [status items](placement.md#shown-when-a-switch-for-the-item).

## Themes

A plugin can ship themes. Each is a file in the package:

```toml
[[contributes.themes]]
id = "night"
file = "themes/night.toml"
```

A theme file is TOML that sets design tokens. The token `color.surface` is the key `surface` in the
table `[color]`:

```toml
name = "Acme Night"
scheme = "dark"

[color]
surface = "#101216"
accent = "#7aa7ff"
```

- `name` is what a person calls the theme.
- `scheme` is `dark` or `light`.
- `inherits` names a theme whose tokens fill in what this one leaves out.
- A theme needs only the tokens it changes. The rest come from the theme it inherits, and in the
  end from the built-in theme of its scheme.

The file is read when the package is installed. A file that is not there, is not TOML, or has
anything wrong in it stops the install, with the problem named.

An installed theme joins the themes the user can choose. Its id is the id the plugin is installed
under, a `/`, and the theme's `id`: `acme.themes/night`. A plugin that only ships themes needs no
code and no `main`.

## Keybindings

A keybinding runs a command when its keys are pressed.

```toml
[[contributes.keybindings]]
key = "ctrl+shift+y"
macos = "cmd+shift+y"
command = "refresh"
```

- `key` is a chord, such as `ctrl+shift+y` or `f7`, or several chords with spaces between them for
  a sequence.
- `macos` gives other keys on a Mac.
- `command` is one of the plugin's own commands, a dependency's, or one of the application's that
  needs no permission.
- `args` is given to the command.
- `when` says where the key holds. See [conditions](placement.md#conditions).

A plugin's keys are defaults. They come after the application's own keys and before the user's: the
user's keybindings in the configuration always win.

### Which keys a plugin may bind

Programs in a terminal expect most keys for themselves. A plugin's binding must begin with a chord
that is one of these:

- Ctrl+Shift with a key, as in `ctrl+shift+y`;
- the logo key (Cmd, Super, Win) with a key;
- a function key, `f1` to `f24`, with or without modifiers.

A key alone, a key with Shift alone, Ctrl with no Shift, and anything with Alt are left to the
programs in the terminal. Only function keys are exempt.

### When a binding is dropped

The manifest's check only reads the keys. Whether a binding takes effect is worked out when the
plugin is loaded. A binding is left out, with a warning, when:

| Why                                                                   | The warning says                     |
| --------------------------------------------------------------------- | ------------------------------------ |
| Its first chord is one programs in a terminal expect                  | which keys a plugin's may begin with |
| Its keys are, begin, or are begun by keys the application binds       | `Lume binds <keys>`                  |
| Its keys are, begin, or are begun by keys the user binds              | `you bind <keys>`                    |
| A plugin that loads earlier bound the same keys, or keys that overlap | `a plugin before it binds <keys>`    |
| Its `when` names something that is not a context                      | what is wrong with the condition     |
| Its command is one of the application's that plugins may not run      | `plugins may not run <command>`      |
| Its command belongs to a dependency that is not installed             | `<command> is not installed`         |

"Begin" matters for sequences: `ctrl+shift+y` and `ctrl+shift+y g` cannot both be bound, because
the first is the start of the second.

A dropped binding does not stop the plugin. The command still runs from the palette and from menus.
The warning is shown with the plugin under **Add-ons** in the settings.

Do not rely on a key. Give every command a title that reads well in the palette, and treat the key
as a shortcut the user may not have.
