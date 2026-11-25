# Permissions

A plugin can do nothing by itself. Its own status items, panels, settings and stored values need no
permission. Everything beyond that is a permission the manifest asks for, often narrowed to a
scope: the files it may read, the programs it may run, the servers it may reach.

The user sees the list in plain words when installing and accepts all of it or none. What was
accepted is kept with the install, and every call the code makes is checked against it. A call
that is not covered fails with the code `denied`.

```toml
[permissions]
"system.metrics" = true
"fs.read" = ["${workspace}/**"]
"process.run" = { scope = ["git"], prompt = "each" }

[permissions.optional]
"net" = ["api.example.com"]
```

## The kinds

| Permission              | Scope         | What it allows                                                                                                 |
| ----------------------- | ------------- | -------------------------------------------------------------------------------------------------------------- |
| `fs.read`               | path patterns | Reading, listing, looking at and watching files and folders.                                                   |
| `fs.write`              | path patterns | Creating, changing and deleting files.                                                                         |
| `process.run`           | program names | Running those programs, with any arguments.                                                                    |
| `process.run-elsewhere` | program names | Running those programs where a session is, on machines the user allows.                                        |
| `net`                   | servers       | Connecting to those servers over HTTP and HTTPS.                                                               |
| `panes.read`            | none          | Reading what the terminals show, their scrollback, and what their programs use of the machine.                 |
| `panes.write`           | none          | Typing into the terminals.                                                                                     |
| `sessions.spawn`        | none          | Starting new terminals: from the user's profiles, or running a program that `process.run` covers.              |
| `workspace.read`        | none          | Seeing the workspace: its windows, tabs and panes, their folders, what runs in them, and what sessions report. |
| `layout.write`          | none          | Opening, closing, moving and splitting tabs and panes.                                                         |
| `clipboard.read`        | none          | Reading the clipboard.                                                                                         |
| `clipboard.write`       | none          | Putting text on the clipboard.                                                                                 |
| `system.metrics`        | none          | Seeing how busy the processor, memory, graphics card and network are.                                          |
| `services.use`          | plugin ids    | Calling the services those plugins offer.                                                                      |

The [API reference](api.md) says which permission each call needs.

## How a permission is written

| Written as                                             | For                                              |
| ------------------------------------------------------ | ------------------------------------------------ |
| `"panes.read" = true`                                  | A kind with no scope.                            |
| `"fs.read" = ["${workspace}/**"]`                      | A kind with a scope: the list of what it covers. |
| `"process.run" = { scope = ["git"], prompt = "each" }` | A scope, and asking before each use.             |
| `"panes.write" = { prompt = "each" }`                  | No scope, and asking before each use.            |

- A kind with a scope needs one. `"fs.read" = true` is an error.
- A kind without a scope takes none.
- `false` is an error. Leave out a permission that is not wanted.
- `prompt` is `"once"`, the default, or `"each"`.

## Scopes

### Paths: `fs.read` and `fs.write`

A path pattern is absolute, or begins with one of three variables:

| Variable         | Stands for                                                    |
| ---------------- | ------------------------------------------------------------- |
| `${workspace}`   | The root folder of the workspace in use, which the user sets. |
| `${home}`        | The user's home folder.                                       |
| `${plugin-data}` | The plugin's own data folder.                                 |

In a pattern:

- `*` matches any run of characters within one name;
- `?` matches any one character;
- `**` matches any number of folders, none included.

So `${workspace}/**` is everything in the workspace, and `${home}/.config/acme/*.toml` is the TOML
files of one folder.

Rules for a pattern:

- A variable only begins a pattern, and is followed by `/` or by nothing.
- No part may be `..`.
- A Windows path may be written with `\` or `/`, and begins with a drive, as `C:/Tools/**`.

The code names files the same way: `lume.fs.read("${workspace}/notes.txt")`.

A workspace with no root folder set has no `${workspace}`. A pattern or a path that begins with it
then matches nothing, and the call is denied.

A plugin's own data folder is not open to it without asking. To keep files there, ask for it:

```toml
[permissions]
"fs.read" = ["${plugin-data}/**"]
"fs.write" = ["${plugin-data}/**"]
```

`lume.kv` and `lume.secrets` need no permission.

### Programs: `process.run`

A program is named by its name alone, as `git`: letters, digits, `.`, `_`, `+` and `-`, never a
path. The name is looked up once along the system's path, never in the current folder, and what was
found is what runs.

On Windows a batch file is run by `cmd.exe`. A plugin that runs `npm`, which is `npm.cmd` there,
needs `cmd` in its scope.

A program is stopped together with everything it started, and at the latest when the plugin stops.

### Programs on other machines: `process.run-elsewhere`

A session need not be on this machine. One that is in a Linux distribution of this Windows machine
(WSL) has a `place` in `sessions.list()`: the machine, and the folder there. Its `cwd` is left out,
because that would be a path of this machine. Panes of servers and of SSH profiles will have a
place in the same way.

`lume.process.run` and `lume.process.spawn` run a program on such a machine when they are given its
id:

```js
for (const session of await lume.sessions.list()) {
  if (session.place?.folder === undefined) continue;
  const { machine, label, folder } = session.place;
  try {
    const result = await lume.process.run("git", ["status"], { machine, cwd: folder });
    lume.log.info(`${label}: ${result.stdout}`);
  } catch (error) {
    // Shown with a button that calls lume.machines.allow(machine).
    if (error.code === "machine_not_allowed") lume.log.info(`not allowed on ${label}`);
    else throw error;
  }
}
```

- The permission is its own: `process.run` allows nothing there, and this one nothing here.
- The user allows each machine. `lume.machines.allow(machine, reason)` asks them, and their yes is
  kept until they revoke it on the plugin's page in the settings. Call it when the user asked for
  it, as by pressing a button. After a no it answers no for a minute without asking again.
- When the plugin comes to ask for more programs, each machine is asked about again.
- A machine can be used while a session is on it.
- The folder is an absolute path of that machine; `${home}` and the like stand for nothing there.
  Left out, it is the user's home folder. In a folder that is not there nothing is run.
- The program is looked up on that machine's path. It is given its whole input when it starts:
  `process.spawn` gives a program there no input to write to.
- A plugin has eight programs at most running on a machine at once; more wait their turn.
- A program ended by a signal has the code its shell gives it, 128 and the signal's number.

| Code                  | What it means                                             | `error.data`         |
| --------------------- | --------------------------------------------------------- | -------------------- |
| `machine_not_allowed` | The user has not allowed the plugin there.                | `{ machine, label }` |
| `not_connected`       | No session is on that machine.                            | `{ machine, label }` |
| `not_found`           | The program or the folder is not there.                   |                      |
| `unavailable`         | The machine could not be reached, or the way to it broke. |                      |

### Servers: `net`

| Written as             | Covers                                                        |
| ---------------------- | ------------------------------------------------------------- |
| `api.example.com`      | That host, on ports 80 and 443.                               |
| `api.example.com:8443` | That host, on that port.                                      |
| `api.example.com:*`    | That host, on any port.                                       |
| `*.example.com`        | Every name under `example.com`, but not `example.com` itself. |
| `192.0.2.7`            | An IPv4 address.                                              |
| `[2001:db8::1]`        | An IPv6 address.                                              |
| `*`                    | Any server.                                                   |

Redirects are followed, five at most, and each one is checked as the first request was. A redirect
to another server does not carry the request's `Authorization` and `Cookie` headers.

### Plugins: `services.use`

The scope lists the plugins whose services may be called, by the ids their manifests give them. See
[services between plugins](#services-between-plugins).

## What is always refused

Some things are refused whatever was granted:

- **Lume's own folders.** Nothing in them can be read or written, even under `${home}/**`. The one
  exception is the plugin's own data folder.
- **Lume's own programs.** They are never run, and neither is any program in Lume's folders.
- **`..` in a path.** A path that tries to go up is not a path.
- **Links that lead out.** A path is followed through every link before it is checked, so a link
  inside the workspace that leads out of it leads nowhere. A file with more than one name (a hard
  link) is never written.
- **This computer and the local link.** A server whose address is this computer's (loopback), of
  the local link, unspecified, multicast or broadcast is refused, even under `*`. This holds for
  the address actually connected to, so a name that leads to `127.0.0.1` is refused too. Such an
  address is allowed only when the scope names it itself, or names `localhost` for this computer.
  Addresses of private networks are not refused.
- **Whether something exists.** A path outside the grants is refused before anything says whether
  it is there.

## What the user is shown

When installing, each permission is put in plain words:

| Permission              | The user reads                                                                                                                                             |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fs.read`               | Read files in: (the patterns)                                                                                                                              |
| `fs.write`              | Create, change and delete files in: (the patterns)                                                                                                         |
| `process.run`           | Run these programs, with any arguments: (the names)                                                                                                        |
| `process.run-elsewhere` | On other machines that your terminals are on (servers, SSH, WSL), each once you have allowed it there: run these programs, with any arguments: (the names) |
| `net`                   | Connect to: (the servers, with their ports)                                                                                                                |
| `panes.read`            | Read what your terminals show, and their scrollback                                                                                                        |
| `panes.write`           | Type into your terminals                                                                                                                                   |
| `sessions.spawn`        | Start new terminals, from your profiles or with a program it may run                                                                                       |
| `workspace.read`        | See your workspace: its windows, tabs and panes, their folders, and what runs in them                                                                      |
| `layout.write`          | Open, close, move and split your tabs and panes                                                                                                            |
| `clipboard.read`        | Read what you copy                                                                                                                                         |
| `clipboard.write`       | Put text on the clipboard                                                                                                                                  |
| `system.metrics`        | See how busy the processor, memory, graphics card and network are                                                                                          |
| `services.use`          | Use what these plugins offer: (the ids)                                                                                                                    |

Three requests come with a warning, because they are close to trusting the plugin with everything:

- `fs.write` over the whole home folder or a whole disk;
- `process.run` of a shell or another program that can do anything given the right arguments:
  `sh`, `bash`, `zsh`, `fish`, `dash`, `ksh`, `cmd`, `powershell`, `pwsh`, `python`, `python3`,
  `node`, `deno`, `bun`, `perl`, `ruby`, `lua`, `git`, `wsl`, `ssh`, `env`;
- `panes.read`, which includes anything typed or printed in a terminal, passwords and keys too.

`process.run-elsewhere` of such a program comes with the same warning when the user is asked about a
machine.

Ask for the narrowest scope that does the job. A user who reads `Read files in: ${workspace}/**`
says yes more readily than one who reads `${home}/**`.

## Asking before each use

`prompt = "each"` has the user asked before every use, shown exactly what would be done. It is
offered for the kinds where that can be shown:

| Permission              | The user is shown                        |
| ----------------------- | ---------------------------------------- |
| `panes.write`           | The text that would be typed, and where. |
| `process.run`           | The command that would be run.           |
| `process.run-elsewhere` | The command, and the machine.            |

```toml
[permissions]
"panes.write" = { prompt = "each" }
```

```js
lume.commands.register("type", async (args) => {
  await lume.panes.write(args.pane, args.text);
  lume.log.info("typed");
});
```

The call waits while the user decides. If they say no, it fails as `denied`. If there is no window
to ask in, or nobody answers, it fails too.

What is approved is shown whole. A text or a command longer than 4000 characters cannot be shown,
so it is refused as `too_large`.

Any other kind written with `prompt = "each"` is an error in the manifest.

## Optional permissions

A plugin that needs something only sometimes can ask for it while it runs, instead of at install.
The manifest lists what it may ask for later:

```toml
[permissions.optional]
"workspace.read" = true
"net" = ["api.example.com", "*.example.org"]
```

| Written as         | Means                                             |
| ------------------ | ------------------------------------------------- |
| `"<kind>" = true`  | The plugin may ask for this kind, over any scope. |
| `"<kind>" = [...]` | It may ask only for scopes from this list.        |

The install prompt lists these under "It may ask you later to". Nothing is granted until the code
asks and the user agrees:

```js
lume.commands.register("request", async () => {
  const granted = await lume.permissions.request(
    "workspace.read",
    undefined,
    "to say which workspace this is",
  );
  if (granted) {
    const workspace = await lume.workspace.current();
    lume.log.info(`workspace ${workspace.name}`);
  }
  return granted;
});
```

`lume.permissions.request(permission, scope?, reason?)`:

- `permission` is a kind the manifest lists under `[permissions.optional]`. Any other is refused as
  `denied`.
- `scope` is what it should cover, written as a manifest writes it. A kind that has a scope must be
  asked for with one. It must be within the list the manifest gave, if it gave one.
- `reason` is shown to the user under the plugin's name. It is 300 characters at most.
- The answer is `true` or `false`. A permission the plugin already holds gives `true` without
  asking.

A permission granted this way is kept with the install, like the ones granted at install. It stays
across updates as long as the new version still lists it as optional.

`lume.permissions.list()` gives the permissions the plugin holds now.

## Asking the user

Three calls put something to the user and need no permission:

| Call                                 | Shows                                 | Limits                             |
| ------------------------------------ | ------------------------------------- | ---------------------------------- |
| `lume.ui.toast(message, level?)`     | A notice.                             | 300 characters.                    |
| `lume.ui.confirm(title, message, o)` | A question with a button to go ahead. | Title 80, message 1000 characters. |
| `lume.ui.pick(items, options?)`      | A list to choose one from.            | 500 items; label 120, detail 200.  |

Each is drawn by Lume in the window in front, under the plugin's name, as plain text. The plugin
cannot draw it, fill it in or dismiss it.

## The application's commands

`lume.commands.execute(id, args)` runs one of the plugin's own commands, by its name, or one of the
application's, by its id. What a plugin may run of the application's depends on the command:

| Commands                                                                                                                                                                                                    | A plugin's code needs |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| Those that start a session: `lume.tab.new`, `lume.window.new`, `lume.terminal.new-with-profile`, `lume.pane.split-right`, `-down`, `-left`, `-up`, and everything under `lume.profile.` and `lume.connect.` | `sessions.spawn`      |
| The rest under `lume.tab.`, `lume.pane.`, `lume.window.`, `lume.group.`, `lume.workspace.` and `lume.panel.`                                                                                                | `layout.write`        |
| Those under `lume.terminal.` and `lume.search.`                                                                                                                                                             | `panes.write`         |
| Everything else, such as `lume.palette.open` or `lume.view.zoom-in`                                                                                                                                         | nothing               |

### Terminals that run a program

`lume.sessions.spawn(profile, { program, args, env })` opens a terminal that runs a program in place
of a profile's shell. It needs two permissions: `sessions.spawn`, and `process.run` over the
program, which is named and found as a program the plugin runs itself is. On Windows a batch file
needs `cmd` in the scope, and takes no argument that is empty or holds a space or one of
`"&|<>^%!()`.

The same holds for `lume.layout.apply` with an intent that opens a terminal (`split`, `tab_new`,
`window_new`, `scratch_show`): `layout.write` alone opens views and moves panes, but a new terminal
needs `sessions.spawn`, and a `launcher` in it must be `{ type: "shell", program, args, login }`
with a program `process.run` covers. Other launchers, and sessions that already run, are refused.

Some commands only the user may run, because they widen what plugins may do or end what the user
has running: `lume.app.quit`, `lume.app.quit-end-sessions`, `lume.workspace.delete`,
`lume.workspace.set-root`, `lume.terminal.copy`, `lume.terminal.paste`, `lume.config.open`,
`lume.window.answer`, and everything under `lume.session.` and `lume.host.`.

A menu item, a status item's button or a keybinding in the manifest can run only the application's
commands of the last row: those that need nothing. The others can be run from code alone, with
their permission.

A plugin cannot run another plugin's commands. It reaches another plugin through its services.

### Verbs of the command line

A [verb](manifest.md#contributescli) a plugin adds to `lume` runs one of that plugin's own
commands, as the palette or a key would. It needs no permission and gives none: the command's code
may do what the plugin may do, and no more, whoever typed the verb. Only the user who runs the
application can reach it from the command line.

## The audit log

Every call that needs a permission is written to an audit log, allowed or refused: the time, the
plugin, the permission, the scope it was checked against and the outcome. What the call read or
wrote is never written.

```sh
lume plugin audit acme.metrics
```

```text
2026-11-19T08:30:05Z  system.metrics    allowed
2026-11-19T08:30:09Z  clipboard.read    denied
```

- Refusals are always written. The same allowed call is written once a minute at most.
- An answer the user gives to `lume.permissions.request` is written too.
- `--json` prints the entries as one JSON object.
- The settings window shows a plugin's latest entries under **Add-ons**.

## Services between plugins

A service is a function one plugin offers and others call. It is how plugins work together.

The plugin that offers it declares it and exports a handler:

```toml
schema = 1
id = "acme.provider"
name = "Provider"
version = "1.0.0"
api = "1"
main = "host.js"
activation = ["onService:double"]

[[contributes.services]]
name = "double"
description = "Doubles a number."
```

```js
lume.services.export("double", (args) => ({ twice: args.n * 2, from: lume.self.id }));
```

The plugin that calls it depends on the first, asks for `services.use` over it, and calls:

```toml
[dependencies]
"acme.provider" = "^1"

[permissions]
"services.use" = ["acme.provider"]
```

```js
const answer = await lume.services.call("acme.provider", "double", { n: 21 });
// answer.twice === 42
```

- The plugin is named by the id its manifest gives it.
- The arguments and the answer are JSON.
- The offering plugin is started to answer, if it is not running.
- The handler may be `async`. It has 30 seconds, not counting the time the user is being asked
  something.
- The call fails as `denied` without `services.use` over that plugin, and as `not_found` when the
  plugin is not installed, cannot be used, or declares no such service.

### When the dependency is bound to another install

The user can bind a dependency to another install, such as a fork installed under an id of its own.
See [binding dependencies](install.md#binding-dependencies). The code does not change: it still
names `acme.provider`, and the call goes to the install that name is bound to.

The grant, though, was given for one install. The first time a call would go to a different one,
Lume asks the user: use what the other install offers, in place of the one named? If they agree,
that is remembered and the call goes through. If they do not, the call fails as `denied`.

`lume.self.id` in the answering plugin is the id it is installed under, which tells a caller which
install answered.
