# Lume

A cross-platform terminal that works standalone or as a client of a Rex server.

Lume is at an early stage: the repository builds, and a hello window reports what the webview
on your machine can do. There is no terminal yet.

## Build

You need Rust (the version in `rust-toolchain.toml` is installed by rustup on first use),
Node (the version in `.node-version`), and on Windows and Linux a libclang for bindgen. Zig
and libghostty-vt are fetched for you at pinned versions; a system Zig is never used.

```sh
cargo xtask setup   # fetch pinned Zig and libghostty-vt, install JS dependencies
cargo xtask build   # build libghostty-vt (native and WASM), the UI, and the workspace
cargo xtask test    # Rust and JS tests
cargo xtask dev     # run the app with hot reload
```

`target/debug/lume-gui --probe` starts the app, prints what the webview supports as JSON,
and exits.

## Layout

| Path            | Contents                                                                                                                                       |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/`       | Rust crates: the app (`lume-app`), the terminal engine bindings (`lume-vt-sys`, `lume-vt`), and placeholders for crates that are not built yet |
| `packages/`     | TypeScript packages: the WASM terminal engine wrapper (`@lume/vt`) and the UI (`@lume/ui`)                                                     |
| `xtask/`        | Build orchestration                                                                                                                            |
| `tools.toml`    | Pinned versions and checksums of external tools                                                                                                |
| `tests/corpus/` | Recorded terminal streams with expected screens                                                                                                |

## Licence

Apache-2.0. See `LICENSE`.
