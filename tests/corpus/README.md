# Golden corpus

Recorded terminal byte streams with the screen each must produce. The recordings are replayed
through the native build of libghostty-vt (`lume-vt`) and through the WebAssembly build
(`@lume/vt`), under Node and inside the application's webview. Each must produce the dump in
`expect.txt`, byte for byte, which is what shows the builds agree.

## A recording

A directory with three files:

| File | Contents |
|---|---|
| `input.bin` | The bytes a program wrote to the terminal |
| `script.toml` | `description`, the initial `cols` and `rows`, and `[[resize]]` entries (`at` = byte offset in the input, `cols`, `rows`) |
| `expect.txt` | The expected dump after all input has been fed: the active area, the state the program left, and what the terminal answered |

## The dump

```text
size 80x24
cursor 3,1 visible
screen primary
--- text
00|hello
01|red
--- attrs
01 0..3 fg=palette:1
--- state
title=vim
pwd=file://host/tmp
modes 7 25 2004
ansi-modes
--- replies
\x1b[?62;22c
```

- `cursor x,y` is zero-based, followed by `visible` or `hidden`, and `pending-wrap` when the
  cursor sits past the last column.
- `screen` is `primary` or `alternate`.
- Text rows have trailing blanks removed. A wide character appears once.
- `attrs` lists runs of adjacent cells that share a non-default style, as
  `row start..end style`, where `start..end` is a half-open column range. Colours are
  `palette:N` or `rgb:rrggbb`.
- `state` holds what a program leaves behind that the screen does not show: the title and
  the working directory it set, and which modes are on. `modes` lists DEC private modes and
  `ansi-modes` ANSI modes, by the number a program uses, out of a fixed list: cursor keys,
  origin, wraparound, cursor shown, mouse reporting, focus events, the alternate screen,
  bracketed paste, synchronized output and a few more; insert and new-line.
- `replies` is one line: every byte the terminal wrote back to the program, in order. This
  is where a hostile stream shows. A title that must not be reported, or a clipboard that
  must not be read, leaves the line without it.
- In `title`, `pwd` and `replies`, bytes outside printable ASCII, and the backslash, are
  written as `\xNN`.

## Running

```sh
cargo test -p lume-vt --test corpus          # the native build
pnpm --filter @lume/vt test                  # the WebAssembly build, in Node
lume-gui --harness corpus                    # the WebAssembly build, in the webview
```

`cargo xtask test` runs the first two. The WebAssembly side needs the module built first
(`cargo xtask vt wasm`). The third needs the application built with its test page
(`cargo xtask build`); `cargo xtask e2e` runs it with the other suites that need a window.

The goldens are written by the native build only. To rewrite them:

```sh
UPDATE_GOLDEN=1 cargo test -p lume-vt --test corpus
```

A golden written this way records what the library did, not what is correct. Read the diff
before keeping it.

## The current recordings

Every recording is authored by `make_synthetic.py`, not captured from a real program: what
each holds is then known line by line, and nothing of the machine it was made on ends up in
it. Each golden was read against its input by hand when it was created.

| Recordings | What they hold |
|---|---|
| `01` to `05` | What a terminal shows: a shell session, colours and attributes, wide characters and emoji, a full-screen program's drawing, reflow on resize |
| `06` to `11` | What programs switch on and ask: scroll regions, keyboard and mouse modes, synchronized output, a shell's reports about itself, an editor starting up, and the questions a program asks its terminal, with the answers |
| `12` to `15` | Hostile streams: going for the clipboard, trying to have a title or a request sent back as input, links that are not what they seem, and numbers and strings past every limit |
| `16`, `17` | A flood through a small screen, and a full reset after a program left state behind |
| `18` | Hostile: the reports of a shell, made by output that is not a shell's. The session host reads the same recording in its scanner's tests, which hold what it passes on and what it drops |

The hostile recordings are also fed to a terminal set up as the product sets it up, in
`packages/ui/test/hostile.test.ts`, which checks what the product's own limits add.

To add a recording, write its stream in `make_synthetic.py`, run the script, write the golden
with `UPDATE_GOLDEN=1`, and read the golden against the stream.
