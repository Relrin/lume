# Lume

A cross-platform terminal with a focus on modularity and Rex server support

The Lume project is a focused on bringing an idea of a terminal that built on a top of libghostty while embracing  
a Pi coding agent approach and allowing to adapt software to user's workflows.

> [!NOTE]
> Lume is a third-party client of the Rex server. It is not made, endorsed or supported by [Superlogical](https://www.superlogical.com/).
> None of the logos and icons of Rex and Superlogical are used.
>
> Keep in mind that this is an unofficial app that implemented for experimenting with features and cover my own needs.
>
> In order to work with Rex server, you would need to be signed up for a beta and install the latest binaries. The
> Rex multiplexer is still in the early access state, so expect breaking changes on the way.

## Features

- The application and core libraries are implemented with Rustlang
- Made on the top of libghostty & WebGPU
- An ability to run in a standalone mode and/or with [Rex server](https://www.superlogical.com/) (optional): on this
  computer, over SSH or over HTTPS. `lume --rex [server]` starts on one
- No built-in coding agents or other tools: a user decides what to use
- Supports addons in TypeScript / JavaScript
- Supports Windows, Linux and macOS

## Development

Pre-requisistes:

- Rust 1.100 (or later)
- NodeJS 24.11.0

In order to simplify the development there is exist the xtask CLI app that can help with installing required
dependencies, run builds and tests:

```sh
cargo xtask setup   # fetch pinned Zig and libghostty-vt, install JS dependencies
cargo xtask build   # build libghostty-vt (native and WASM), the UI, and the workspace
cargo xtask test    # Rust and JS tests
cargo xtask dev     # run the app with hot reload
```

`target/debug/lume-gui --probe` starts the app, prints what the webview supports as JSON,
and exits.

## Project layout

| Path            | Contents                                                                                                                                       |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/`       | Rust crates: the app (`lume-app`), the terminal engine bindings (`lume-vt-sys`, `lume-vt`), and placeholders for crates that are not built yet |
| `packages/`     | TypeScript packages: the WASM terminal engine wrapper (`@lume/vt`) and the UI (`@lume/ui`)                                                     |
| `xtask/`        | Build orchestration                                                                                                                            |
| `tools.toml`    | Pinned versions and checksums of external tools                                                                                                |
| `tests/corpus/` | Recorded terminal streams with expected screens                                                                                                |

## License

The Lume project is published under the Apache-2.0 license. For more details read the [LICENSE](LICENSE) file.
