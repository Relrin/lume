# Golden corpus

Recorded terminal byte streams with the screen each must produce. The recordings are replayed
through the native build of libghostty-vt (`lume-vt`). The WebAssembly build (`@lume/vt`)
will replay them too, and both must produce the same dump.

## A recording

A directory with three files:

| File | Contents |
|---|---|
| `input.bin` | The bytes a program wrote to the terminal |
| `script.toml` | `description`, the initial `cols` and `rows`, and `[[resize]]` entries (`at` = byte offset in the input, `cols`, `rows`) |
| `expect.txt` | The expected dump of the active area after all input has been fed |

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
```

- `cursor x,y` is zero-based, followed by `visible` or `hidden`, and `pending-wrap` when the
  cursor sits past the last column.
- `screen` is `primary` or `alternate`.
- Text rows have trailing blanks removed. A wide character appears once.
- `attrs` lists runs of adjacent cells that share a non-default style, as
  `row start..end style`, where `start..end` is a half-open column range. Colours are
  `palette:N` or `rgb:rrggbb`.

## Running

```sh
cargo test -p lume-vt --test corpus
```

To rewrite the goldens:

```sh
UPDATE_GOLDEN=1 cargo test -p lume-vt --test corpus
```

A golden written this way records what the library did, not what is correct. Read the diff
before keeping it.

## The current recordings

The five recordings here are authored by `make_synthetic.py`, not captured from real
programs. Each golden was read against the input by hand when it was created. Captures of
real shells and full-screen programs will be added once Lume has a PTY to record from.
