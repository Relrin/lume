# Writing plugins for Lume

A plugin is a package: a folder, or a `.tar.gz` archive, with a manifest called `plugin.toml` at
its root and, if it needs any, one JavaScript file of code.

The manifest says what the plugin is, what it adds to Lume and what it needs. Lume reads it before
any code runs. Menu items, status items, settings, panels, keys, themes, pictures and effects all
come from the manifest alone.

The code runs in a sandboxed JavaScript engine, one engine for each plugin. It has the JavaScript
standard library, a global `lume` object and `console`. It has no files, no network, no processes
and no modules of its own. Everything it does beyond its own memory is a call to `lume`, and Lume
checks each call against the permissions the user granted.

Most of what a plugin shows is drawn by Lume from data the plugin gives: a status item's value, a
list panel's rows, a question's text. A plugin that needs to draw for itself adds a view: a page of
its own in a panel or a pane, shown in a sandboxed frame that can reach nothing but the plugin's
own code.

## Pages

| Page                                  | What it covers                                                                                          |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| [Getting started](getting-started.md) | A first plugin from nothing, in plain JavaScript and then in TypeScript.                                |
| [The manifest](manifest.md)           | Every key of `plugin.toml`, with its type, default and limits.                                          |
| [The environment](environment.md)     | What the code can use, the budgets it is held to, logs, errors.                                         |
| [Placement](placement.md)             | Menus, sections, submenus, status items and panels, and how they are ordered.                           |
| [Settings](settings.md)               | Settings and settings pages, themes, keybindings.                                                       |
| [Pictures and motion](motion.md)      | Pictures, motion for the window's own places, cursor effects drawn by shaders.                          |
| [Views](views.md)                     | Pages of the plugin's own in panels and panes: what they may load, how they reach the code.             |
| [The plugin kit](kit.md)              | `@lume/plugin-kit`: the client, hooks and controls a view is built with, and the template.              |
| [Developing](develop.md)              | Linking a folder, what is loaded again as files change, looking into a view.                            |
| [Permissions](permissions.md)         | Each permission and its scope, per-use approval, the audit log, services.                               |
| [Installing](install.md)              | Packages, repositories, the install prompt, updates, side-by-side installs, the `lume plugin` commands. |
| [Performance](performance.md)         | How to write a plugin that costs nothing while it has nothing to do.                                    |
| [API reference](api.md)               | Every call of the `lume` object and every type, generated from Lume itself.                             |

## Names

A plugin's id is lower-case words joined by dots, as in `acme.metrics`. It is the plugin's
namespace everywhere:

- its commands are `acme.metrics.<name>`;
- its settings are under `[plugins."acme.metrics"]` in the configuration;
- its stored data is kept under the same id.

Inside its manifest and its code a plugin names its own things without that prefix: `refresh`, not
`acme.metrics.refresh`. The id a plugin is installed under may not be the one its author gave it
(see [side-by-side installs](install.md#side-by-side-installs)), so the short name is the one that
always works.

No plugin may be called `lume` or have an id that begins with `lume.`. Everything under `lume.` is
the application's own.
