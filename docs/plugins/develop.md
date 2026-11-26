# Developing a plugin

An installed plugin is a copy that Lume holds to a hash, so editing it does nothing. While a plugin
is being worked on, **link** its folder instead: Lume uses the folder where it lies and loads what
changes as it is saved.

## Linking a folder

```sh
lume plugin link ./notes
```

```text
acme.notes is linked to /home/me/notes, and is given what its manifest asks for. Lume follows the folder: a change to its files is loaded as it is saved.
```

| Flag            | What it does                                                                        |
| --------------- | ----------------------------------------------------------------------------------- |
| `--as <id>`     | Links under another id, as [`install --as`](install.md#side-by-side-installs) does. |
| `--name <name>` | Gives the link a name of the user's.                                                |

A linked folder differs from an installed package:

- Nothing is copied. The manifest's paths are read from the folder, so a manifest that names
  `dist/host.js` needs the build to have run.
- Its files are not held to a hash.
- It is given what its manifest asks for without a prompt, and again whenever the manifest changes.
  Linking is for code you are writing; do not link a folder you would not install.
- It is marked **Dev** wherever it is named: in `lume plugin list`, in the settings, and in the
  headers of its panels and panes.
- `.git`, `.hg`, `.svn` and `node_modules` are not looked at, wherever they are in the folder.

A link takes the place of a link under the same id. It does not take the place of an installed
package: remove that first, or link beside it with `--as`.

`lume plugin remove <id>` unlinks. The folder is not touched.

Installing a package over a link asks about every permission, as a new install does.

## What is loaded again

A running Lume watches each linked folder. A moment after the last file of a change was written, it
loads again what the change calls for:

| What changed                                                                    | What happens                                                                                                       |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `plugin.toml`                                                                   | The manifest is read again: what the plugin adds and what it may do follow it. Its code and its views start again. |
| The plugin's code (`main`), or the map beside it                                | The code is stopped and started again. Open views stay as they are.                                                |
| A file one of its views has loaded, or a file in the folder of one of its pages | Its open views are loaded again, each from the [state](views.md#state) it kept.                                    |
| A picture, theme or shader the manifest names                                   | What the plugin adds is worked out again.                                                                          |
| Anything else, such as a source file                                            | Nothing. The build's output is what counts.                                                                        |

The code is started again where it would be running: it was running, it starts with the
application, or one of its views is open. Views that are open when the code starts are told to it
with `lume.views.onOpen`, as when it starts for any other reason.

A change is followed in well under a second. The plugin's log says what was done:

```sh
lume plugin logs acme.notes
```

```text
2026-11-20T02:07:33Z info loaded again after a change to its code, its pages (241 ms, code started)
```

A manifest with a mistake in it keeps the plugin from being used until it is fixed.
`lume plugin list` says what is wrong, and the plugin comes back when the manifest reads again.

### With a build that watches

Run the build in watch mode beside Lume, and every save is on screen a moment later. With the
lume-addons template:

```sh
pnpm --filter @lume-addons/notes exec vite build --watch
pnpm --filter @lume-addons/notes exec vite build --watch --config vite.view.config.ts
```

A build that empties its output folder before it writes makes Lume load the plugin twice, once for
the files that went and once for those that came. That is harmless; a build that writes in place
avoids it.

## Starting from the template

The lume-addons repository has a template with a frame panel, a pane view, the plugin's code and the
builds for both. See [the plugin kit](kit.md#starting-from-the-template).

## Looking into a view

In a build of Lume made for development, the command **Inspect the Views of Linked Plugins**
(`lume.view.inspect`) opens the window's developer tools. Each view of a plugin is a page of its
own there: choose its frame in the tools' list of pages to see its console, its elements and what
it loaded.

The command is there only in development builds, and only while a plugin is linked. Released
builds have no developer tools.

Without them, a view can still be followed from outside:

- `lume plugin logs <id>` shows what the plugin's code logged. Have the view report what it does
  with a call to the code.
- `lume ctl plugins` shows whether the code is running, how often it started, and for a linked
  plugin how often it was loaded again and how long the last time took.
- A view that stopped says why in its place in the window.

## From a link to a release

A linked folder and an installed package are read the same way, so what works linked works
installed, with two things to check before publishing:

- **The package is built.** A package installed from a repository is used as it is fetched, and
  nothing is built on the user's machine. Commit the built files the manifest names.
- **The permissions are the ones you mean.** A link was never asked about. Run
  `lume plugin install ./notes --as acme.notes-check` and read the prompt the user will see.

See [installing](install.md#from-a-repository) for how users install from a repository and get
updates.
