# Renderer snapshots

Reference pictures for the renderer's snapshot suite. The suite draws a fixed set of frames
with every renderer tier the webview offers (WebGPU, WebGL2, Canvas 2D), compares the tiers
with one another, and compares each picture with the reference kept here.

## What is kept

One directory for each webview engine, named after it in lower case:

| Directory   | Engine    | Platform |
| ----------- | --------- | -------- |
| `webview2`  | WebView2  | Windows  |
| `wkwebview` | WKWebView | macOS    |
| `webkitgtk` | WebKitGTK | Linux    |

Text is drawn by the system's rasteriser, so a picture belongs to the engine that drew it.
Within one engine the tiers draw the same picture to within two levels of a colour channel,
which the suite checks pixel by pixel, so one reference for each frame serves all three.

Not every frame has a reference:

- Frames with text the test font lacks (CJK, emoji) are drawn with whatever fonts the
  machine has. Their pictures differ from machine to machine, so for them the tiers are
  only compared with one another.
- The full screens drawn for timing are megabytes as pictures and show nothing the smaller
  frames do not.

The frames are drawn with `tests/fonts/JetBrainsMono-Regular.ttf` at a device pixel ratio of
1, whatever the display's is.

## How a picture is compared with its reference

In blocks of four by four pixels. A block differs when the mean of a colour channel over it
is more than 24 levels from the reference's, and a picture fails when more than half a per
cent of its blocks differ. A letter that is missing, moved or miscoloured changes its block
several times over; the softness of an edge, which varies between machines and between
versions of a webview, does not. The numbers are in `packages/ui/src/harness/tuning.ts`.

## Running

```sh
cargo xtask build
target/debug/lume-gui --harness snapshots --references tests/snapshots --out target/harness/snapshots
```

`cargo xtask e2e` runs it with the other suites that need a window. The exit status is 0
when every frame passed. When a frame fails, what each tier drew is written to
`<out>/failed/<tier>/`.

An engine with no directory here is not a failure: the suite reports the frames as
unreferenced and writes what it drew to `<out>/references/<engine>/`, which is what a
reference would be made from.

## Making or replacing references

```sh
target/debug/lume-gui --harness snapshots --references tests/snapshots --update
```

This writes this engine's references from what the first tier drew. A reference written
this way records what the renderer did, not what is correct. Look at each picture before
keeping it.
