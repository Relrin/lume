# Getting started

This page builds a small plugin from nothing: one command that counts how often it is run, stores
the count and shows it in the top bar. The first version is plain JavaScript. The second half sets
up TypeScript.

## The folder

A plugin is a folder with a manifest and, here, one file of code:

```text
hello/
  plugin.toml
  host.js
```

## The manifest

`plugin.toml`:

```toml
schema = 1
id = "acme.hello"
name = "Hello"
version = "1.0.0"
description = "Counts how often it is greeted."
api = "1"
main = "host.js"
activation = ["onCommand:greet"]

[[contributes.commands]]
id = "greet"
title = "Greet"

[[contributes.status]]
id = "count"
slot = "topbar.right"
kind = "badge"
label = "Greetings"
```

What each part says:

- `schema`, `id`, `name`, `version` and `api` are needed in every manifest.
- `main` names the code. A plugin with commands needs it.
- `activation` says when the code starts. Here it starts when the command `greet` is first run, and
  not before.
- `[[contributes.commands]]` adds a command. Its full id is `acme.hello.greet`.
- `[[contributes.status]]` adds a status item to the right of the top bar. Lume shows its label
  from the manifest before the code has ever run.

The manifest is strict. A key Lume does not know is an error, with the line and column it is on.
[The manifest](manifest.md) lists every key.

## The code

`host.js`:

```js
// Counts how often it is greeted: kept with lume.kv, shown in a status item.
lume.commands.register("greet", async () => {
  const count = ((await lume.kv.get("count")) ?? 0) + 1;
  await lume.kv.set("count", count);
  await lume.status.set("count", { text: String(count) });
  lume.log.info("greeted", count);
  return count;
});
```

The file is run once, from top to bottom, when the plugin starts. Register every command handler
there, before the code waits for anything.

- `lume.commands.register` takes the command's name as the manifest gives it.
- `lume.kv` stores JSON values that belong to the plugin. It needs no permission.
- `lume.status.set` shows a value in one of the plugin's own status items. It needs no permission
  either.
- `lume.log.info` writes a line to the plugin's log. `console.log` does the same.

Every call of `lume` that reaches Lume returns a promise.

## Install it

```sh
lume plugin install ./hello
```

Lume copies the folder, checks it, and says what installing would do:

```text
Install Hello 1.0.0 (acme.hello), from /home/me/hello.
Counts how often it is greeted.

It asks for no permissions.

Go ahead? [y/N]
```

Answer `y`. Nothing is installed until you do. `--yes` answers ahead, for scripts.

A running Lume follows the change by itself. There is no need to start it again.

```sh
lume plugin list
```

```text
In the order they load in:
  acme.hello  Hello 1.0.0  ready
```

## Run the command

Open the command palette and choose **Greet**. It is listed under the plugin's name. The badge in the top bar shows `1`, then `2` the next time.

From a shell, with Lume running:

```sh
lume ctl command acme.hello.greet
```

## Read its log

```sh
lume plugin logs acme.hello
```

```text
2026-11-19T08:30:05Z info greeted 1
```

`--lines <n>` shows the last `n` lines instead of the last 40. Errors the code throws are written
here too, with their stacks.

## See it in the settings

Open the settings and choose **Add-ons**. The plugin is listed there with its version, what it may
do, how it fares, the end of its log and its latest audit entries. From there it can be turned off
and on, removed, and moved in the order plugins load in.

## Change it

An installed package is a copy. Lume keeps a hash of it and will not run a copy whose files have
changed. So after every change to the source, install the folder again:

```sh
lume plugin install ./hello --yes
```

The same plugin installed again replaces the copy and keeps its data, its settings and whether it
was turned on. See [Installing](install.md#updating).

## The TypeScript route

Lume runs one plain script. It has no module loader, so `import` and `require` do not exist when
the code runs. Write TypeScript in as many files as you like, then bundle them into one file.

### Typings

The package `@lume/plugin-api` holds the typings of the global `lume` object and `console`. It is
generated from Lume's own definitions of the API. It is part of the Lume repository, in
`packages/plugin-api`, and is not published to a registry: pack it from there, or depend on it by
path.

`package.json`:

```json
{
  "name": "acme-hello",
  "version": "1.0.0",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "vite build",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "@lume/plugin-api": "file:./vendor/lume-plugin-api-1.0.0.tgz",
    "typescript": "6.0.3",
    "vite": "8.3.8"
  }
}
```

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022"],
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true,
    "types": ["@lume/plugin-api"]
  },
  "include": ["src"]
}
```

`lib` leaves out `DOM` on purpose. The engine has no DOM, and leaving it out makes the compiler say
so when the code reaches for `window`, `fetch` or `setTimeout`.

### One file out

`vite.config.ts`:

```ts
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    minify: false,
    sourcemap: true,
    lib: {
      entry: "src/host.ts",
      formats: ["iife"],
      name: "hello",
      fileName: () => "host.js",
    },
  },
});
```

The format is `iife`: a plain script with everything it imports bundled in. `sourcemap: true`
writes `dist/host.js.map` beside the bundle.

### The source

`src/host.ts`:

```ts
import type { Json } from "@lume/plugin-api";

function next(stored: Json): number {
  return (typeof stored === "number" ? stored : 0) + 1;
}

lume.commands.register("greet", async () => {
  const count = next(await lume.kv.get("count"));
  await lume.kv.set("count", count);
  await lume.status.set("count", { text: String(count) });
  return count;
});
```

Point the manifest at the bundle:

```toml
main = "dist/host.js"
```

### Ship the source map

When a file called `<main>.map` is in the package beside the bundle (here `dist/host.js.map`), Lume
writes error stacks in the plugin's log with the source's files and lines instead of the bundle's.
The map is read only when a stack first needs it.

### Build and install

```sh
pnpm install
pnpm build
lume plugin install .
```

Installing a folder leaves out `node_modules`, `.git`, `.hg` and `.svn`, so the working folder can
be installed as it is. To hand the plugin to someone else, pack only what it needs:

```sh
tar -czf hello.tar.gz plugin.toml dist
```

`lume plugin install hello.tar.gz` takes the archive too.

## Where next

- [The manifest](manifest.md) for everything a plugin can add.
- [Permissions](permissions.md) before the code reads a file, runs a program or talks to a server.
- [Performance](performance.md) for choosing activation events.
- [API reference](api.md) for every call.
