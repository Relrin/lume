# Installing and managing plugins

Plugins are installed and managed with `lume plugin`, or under **Plugins** in the settings. The
commands work on what is installed and need no Lume running. A running Lume sees what they change
and follows it without being started again.

| Command                                               | What it does                                      |
| ----------------------------------------------------- | ------------------------------------------------- |
| `lume plugin install <folder or archive>`             | Installs a package, or updates an installed one.  |
| `lume plugin install <repository>`                    | The same, from a repository fetched with git.     |
| `lume plugin update [<id>]`                           | Looks in the repository for something newer.      |
| `lume plugin link <folder>`                           | Uses a folder where it lies, while working on it. |
| `lume plugin list`                                    | Lists what is installed, in the order it loads.   |
| `lume plugin enable <id>`, `lume plugin disable <id>` | Turns a plugin on or off.                         |
| `lume plugin remove <id>`                             | Removes a plugin.                                 |
| `lume plugin rename <id> <name>`                      | Gives a plugin a name of the user's.              |
| `lume plugin order <id> <where>`                      | Moves a plugin in the order plugins load in.      |
| `lume plugin bind <id> <dependency>=<installed id>`   | Binds a dependency to another install.            |
| `lume plugin logs <id>`                               | Shows the end of a plugin's log.                  |
| `lume plugin audit <id>`                              | Shows what a plugin was allowed and refused.      |

`<id>` is always the id the plugin is installed under.

Every command takes `--json`, which prints one JSON object in place of words: `"ok": true` with
what was done, or `"ok": false` with the error. A command that did what it was asked exits with 0,
and one that was refused or failed with 1.

## Packages

A package is a folder, or a `.tar.gz` (or `.tgz`) archive, with `plugin.toml` at its root. An
archive may hold the package itself, or one folder that holds it, as an archive of a repository
does.

Lume copies the package into a folder of its own before it does anything else with it. Nothing in
the package is run during an install.

### What is left out

These folders are never copied, wherever they are in the package:

- `.git`, `.hg`, `.svn`
- `node_modules`

So a working folder can be installed as it is. A plugin's code must be bundled into its own files:
it cannot use anything from `node_modules` when it runs. The install prompt lists what was left
out.

### What is refused

Each of these stops the install:

| In the package                                                                                                                                                       | Why                                            |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| A link, a device, or anything that is not a plain file or folder                                                                                                     | The copy would not be what the user looked at. |
| A path in an archive that is absolute or climbs out with `..`                                                                                                        | It leaves the package.                         |
| A name Windows cannot hold: one with `<`, `>`, `:`, `"`, `\|`, `?`, `*` or `\`, one that ends with a dot or a space, or a device name such as `con`, `nul` or `com1` | The package must be the same on every system.  |
| Two names that differ only in case                                                                                                                                   | Not every system can hold both.                |
| More than 4000 files, or more than 128 MiB unpacked                                                                                                                  | A package holds built files only.              |
| No `plugin.toml`                                                                                                                                                     | It is not a package.                           |

After the copy, the manifest is read and checked, and every file the manifest names is checked
too: the code is there, pictures and shaders keep their rules, theme files read without problems.
All the problems found are listed together.

## From a repository

A package can be installed straight from a git repository:

```sh
lume plugin install https://example.com/acme/metrics.git
lume plugin install acme/metrics                 # a repository on GitHub
lume plugin install acme/tools#v1.2 --path plugins/metrics
```

| Part              | What it is                                                                |
| ----------------- | ------------------------------------------------------------------------- |
| the address       | Anything `git fetch` takes over https or ssh, or `owner/repo` for GitHub. |
| `#<ref>`, `--ref` | A branch or a tag to take. Without one, the repository's default branch.  |
| `--path <folder>` | The folder of the repository the package is in, when that is not its top. |

Lume fetches with the `git` on this machine, so whatever lets you fetch the repository yourself
(keys, a credential helper) works here too. It fetches one commit without its history, checks it
out in a folder of its own, and copies the package from there as it copies any folder. Nothing of
the repository is run: its hooks are not, submodules are not followed, and git may not stop to ask
for a password.

A name that is a folder or an archive on this machine is always taken as that. `--path` and `--ref`
go with a repository only.

The package is installed **as it is fetched**. Nothing is built, so a repository must hold the
built files its manifest names.

The install is pinned to the commit that was fetched, beside its [content hash](#the-content-hash).
The prompt names the commit, and `lume plugin list` shows it:

```text
  acme.metrics  Metrics 1.0.0  ready  [https://example.com/acme/metrics.git at 4f9d2c81a07b]
```

### Updates

```sh
lume plugin update                 # every plugin that came from a repository
lume plugin update acme.metrics
```

`update` fetches the repository again, at the branch or tag the install follows. When the commit
and the files are the ones installed, it says so and does nothing. Otherwise it shows the prompt of
an [update](#updating) (the versions, the new commit, and what the plugin would newly be able to
do) and asks.

`--yes` agrees ahead to an update that asks for nothing new. An update that would let the plugin do
more than the user agreed to is **always** asked about: with nobody to ask, it is left alone, and
the command says what it would have asked.

A plugin that followed a tag stays on that tag. To move to another, install from the repository
again with the new `#<ref>`.

## The install prompt

```sh
lume plugin install ./metrics
```

```text
Install Metrics 1.0.0 (acme.metrics), from /home/me/metrics.
Meters for this machine in the status bar.

It will be able to:
  - Run these programs, with any arguments: git
  - See how busy the processor, memory, graphics card and network are

It may ask you later to:
  - Connect to servers

Note: A program such as a shell or git can do anything you can, so running it is close to trusting this plugin with your account.

Left out of the package: node_modules

Go ahead? [y/N]
```

The prompt shows:

- what is being installed, under which id, and from where;
- the manifest's description;
- every permission, in plain words;
- the optional permissions the plugin may ask for later;
- a note for each permission that comes close to trusting the plugin with everything;
- the folders that were left out.

Nothing is installed until the answer is `y` or `yes`. Any other answer installs nothing.

| Flag            | What it does                                                                    |
| --------------- | ------------------------------------------------------------------------------- |
| `--yes`, `-y`   | Agrees ahead, for scripts.                                                      |
| `--as <id>`     | Installs under another id. See [side-by-side installs](#side-by-side-installs). |
| `--name <name>` | Gives the install a name of the user's. See [display names](#display-names).    |

Where nobody can answer (the command is not run in a terminal, or `--json` is given), the install
is refused unless `--yes` is given.

## The content hash

When a package is installed, Lume records a hash of it. The hash covers every file of the package
and nothing else: each file's path inside the package, its length and the SHA-256 of its bytes. A
file added, removed, renamed or changed gives another hash. So does a link or a device appearing in
the folder.

Lume checks the hash when it reads the installed plugins, and again just before it starts a
plugin's code. A package that no longer matches is not used:

```text
acme.metrics  Metrics 1.0.0  not used: its files have changed since it was installed; install it again to use it
```

The user is told once. Nothing of the plugin is shown and none of its code runs until it is
installed again.

This is why editing an installed copy does not work. While a plugin is being worked on, link its
folder instead of installing it: see [developing](develop.md).

## Updating

To update a plugin, install the new package. An install under an id that is taken is an update when
the package is the same plugin, and is refused when it is not.

Two packages are the same plugin when their manifests give the same `id` and:

- both give a `repository`, and it is the same one (upper and lower case, a trailing `/` and a
  trailing `.git` do not count); or
- neither gives a `repository`, and they give the same `name`.

Set `repository` in every manifest that is meant to be updated. It is what says a new package is
the next version of the old one, and not something else that took its id.

The prompt for an update is headed by what changes:

| The new version is | The heading says                      |
| ------------------ | ------------------------------------- |
| newer              | `Update Metrics from 1.0.0 to 1.1.0`  |
| the same           | `Install Metrics 1.0.0 again`         |
| older              | `Go back from Metrics 1.1.0 to 1.0.0` |

Going back to an older version is allowed, with a note.

An update asks only about what is new. Permissions the plugin already holds are not listed again;
the prompt says `It will also be able to:` with the new ones, or `It asks for nothing new.` A
permission that was granted with asking before each use does not cover the same permission without
asking, so dropping `prompt = "each"` counts as new.

What an update keeps:

- whether the plugin was turned on or off;
- the name the user gave it;
- the bindings of its dependencies;
- its stored data and its settings;
- optional permissions the user granted while it ran, where the new version still lists them as
  optional.

When the new package differs from the old one, a running Lume stops the plugin's code. It starts
again from the new package when it is next wanted.

A package that is another plugin under the same id is refused:

```text
lume: acme.metrics is installed already, from https://example.com/acme/metrics; this package is another plugin of the same id. Install it beside the other under a name of its own, with --as <id>
```

## Side-by-side installs

`--as <id>` installs a package under an id other than its manifest's. That is how two plugins with
the same id, such as an original and a fork, are installed beside each other.

```sh
lume plugin install ./metrics-fork --as me.metrics
```

Everything of the install then goes by the new id:

- its commands are `me.metrics.<name>`;
- its settings are under `[plugins."me.metrics"]`;
- its data and its log are kept under `me.metrics`;
- `lume.self.id` in its code is `me.metrics`, and `lume.self.manifestId` is still `acme.metrics`.

This is why a manifest and its code name the plugin's own things without the id. Code that wrote
`acme.metrics.refresh` anywhere would break under another id.

The id given to `--as` follows the rules of every plugin id. Two installs cannot have ids where one
is inside the other, as `acme` and `acme.tools` are: their commands and settings could take each
other's names.

`lume plugin list` shows where such an install came from:

```text
  me.metrics (from acme.metrics)  Metrics 1.0.0  ready
```

## Display names

`name` in the manifest is what the plugin is called until the user says otherwise.

```sh
lume plugin rename me.metrics "Metrics (my fork)"
lume plugin rename me.metrics --reset
```

A name is one line of 64 characters at most. `--reset` goes back to the manifest's name.
`lume plugin install --name <name>` gives the name at install.

The name is what the user sees: on the plugin's settings page, over its questions and notices, and
as the heading of its commands in the palette when they give no `category`.

## Load order

Plugins load in an order. It decides which of two things comes first where nothing else does: in a
menu, when two plugins' items have the same `order`; among keybindings, when two plugins want the
same keys.

The order is worked out like this:

1. A plugin loads after every plugin it depends on.
2. A plugin loads before those its manifest lists under `[load] before`, where they are installed.
3. Where neither decides, the user's order does.
4. Then the ids, in alphabetical order.

The user's order is changed with `lume plugin order`:

```sh
lume plugin order acme.git --first
lume plugin order acme.git --last
lume plugin order acme.git --before acme.metrics
lume plugin order acme.git --after acme.metrics
```

The command prints the order that results. A plugin installed for the first time goes last.

### Dependencies

A plugin whose manifest lists `[dependencies]` is used only when they are there:

| The dependency is                              | The plugin is                                  |
| ---------------------------------------------- | ---------------------------------------------- |
| not installed                                  | not used, unless the dependency is `optional`. |
| installed at a version the requirement refuses | not used, optional or not.                     |
| turned off, changed, or not usable itself      | not used, unless the dependency is `optional`. |

`lume plugin list` says why:

```text
  acme.git  Git 1.0.0  not used: it needs acme.core, which is not installed
```

When a plugin's code is started, the code of the plugins it depends on is started first.

### Circles

Plugins that depend on each other in a circle are none of them used. Each says so.

A circle made only with `[load] before` stops nothing. The `before`s that make it are set aside,
with a warning, and the plugins load by the rest of the rules.

## Binding dependencies

A manifest names a dependency by the id the other plugin's manifest gives it. By default that name
means the install under that id. The user can bind it to another install:

```sh
lume plugin install ./core-fork --as me.core
lume plugin bind acme.git acme.core=me.core
```

From then on, everything in `acme.git` that names `acme.core` means `me.core`: the dependency
itself and its version requirement, references such as `acme.core:open` in menus, and calls to its
services. Nothing in `acme.git` changes.

```sh
lume plugin bind acme.git acme.core=
```

An empty right-hand side undoes the binding.

- The plugin must list the dependency in its manifest.
- The install it is bound to must be installed, and cannot be the plugin itself.
- The first call to a service of the newly bound install asks the user. See
  [services](permissions.md#when-the-dependency-is-bound-to-another-install).

`lume plugin list` shows bindings under the plugin, as `uses me.core for acme.core`.

## Turning off and on

```sh
lume plugin disable acme.metrics
lume plugin enable acme.metrics
```

A plugin that is turned off stays installed. Nothing it adds is shown and its code does not run.
Its data, its settings and its permissions are kept.

Lume turns a plugin off itself when its code goes over its limits three times in a minute. See
[a breach](environment.md#a-breach).

## Removing

```sh
lume plugin remove acme.metrics
lume plugin remove acme.metrics --purge
```

`remove` takes away the installed package. The plugin's data is kept, for an install under the same
id later. `--purge` removes its data and its log too.

The plugin's table in the configuration is the user's file, and neither form changes it. Lume says
nothing about a table whose plugin is not installed.

## Listing

```sh
lume plugin list
```

```text
In the order they load in:
  acme.core  Core 1.2.0  ready
  acme.git  Git 1.0.0  ready
      uses me.core for acme.core
      note: acme.git:extra is placed by its order: `lume:nowhere` is not beside it here
  acme.metrics  Metrics 1.0.0  not used: it is turned off
```

Each line gives the id, the name, the version, and `ready` or why the plugin is not used. `note:`
lines are warnings about things that could not be placed or read as the manifest asks. See
[what cannot be placed](placement.md#what-cannot-be-placed).

## In the settings

**Plugins** in the settings window shows every installed plugin in the order they load in, with:

- its name, version, description and where it was installed from;
- whether it is turned on, and whether it can be used, with the reason if not;
- its dependencies and what each is bound to;
- what it may do, in plain words, and what it may ask for later;
- how it fares: whether its code is running, its recent breaches, its last problem, its warnings;
- the end of its log and its latest audit entries.

From there a plugin can be:

- installed from a folder or from a repository (**Install from git**), with the same check and the
  same question as `lume plugin install`;
- looked for a newer one of, where it came from a repository (**Check for update**);
- turned on and off;
- removed, keeping its data;
- moved earlier or later in the user's order.

A package whose id is taken by another plugin is refused there too, with an id suggested to install
it under beside the first.

A plugin whose folder is linked is marked **Dev** there, and says which folder.

Renaming, binding, linking and purging are done with `lume plugin`.
