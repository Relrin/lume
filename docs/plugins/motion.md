# Pictures, motion and cursor effects

A plugin never gives the window styles or markup of its own. What it can bring is data that Lume
checks and then draws itself:

- [pictures](#pictures), shown as icons;
- [motions](#motions), effects for fixed places in the window, written as keyframes;
- [cursor effects](#cursor-effects), drawn by a small shader.

All three are declared in the manifest and need no code.

## Pictures

```toml
[[contributes.images]]
id = "gauge"
file = "icons/gauge.svg"
tint = true
```

A picture declared this way has a name. The plugin's other things and its code use the name
wherever a picture is wanted:

- `icon` of a menu item, a status item or a panel in the manifest;
- `icon` of a status value given to `lume.status.set`;
- `icon` of a panel row.

An `icon` in the manifest can be written three ways:

| Written as        | Means                                            |
| ----------------- | ------------------------------------------------ |
| `gauge`           | One of the plugin's own pictures, by its name.   |
| `icons/gauge.svg` | A picture file in the package, by its path.      |
| any other name    | One of the application's own icons, by its name. |

### Kinds and sizes

| Kind                                     | Limit for one file |
| ---------------------------------------- | ------------------ |
| `.svg`                                   | 16 KiB             |
| `.png`, `.jpg`, `.jpeg`, `.webp`, `.gif` | 128 KiB            |

All of a plugin's pictures together may hold 512 KiB.

A file must be the kind its name says: a `.png` that does not begin as a PNG is refused.

An SVG is drawn as an image, so it runs no script and loads nothing. To keep that certain, an SVG
is refused if it holds any of these:

- `<script`, `<foreignObject`, `javascript:` or `<!ENTITY`;
- an `href` that is not to one of its own parts, as `href="#part"`;
- a `url(...)` that is not to one of its own parts, as `url(#part)`.

A picture that breaks a rule stops the install, and the message names the file.

### Tint

With `tint = true` only the picture's shape is used. It is drawn in the colour of what it stands
beside, as the application's own icons are, so it follows the theme. Use it for one-colour icons.
Leave it off for pictures that have colours of their own.

## Motions

A motion is an effect for one of the window's own places. The user chooses it in the settings.
Until they do, the place moves as Lume's own design has it.

```toml
[[contributes.motions]]
id = "pop"
title = "Pop"
slot = "menu.enter"
duration = 180
easing = "emphasized"
keyframes = [
  { opacity = 0.0, transform = "translateY(-4px) scale(0.96)" },
  { opacity = 1.0, transform = "none" },
]
```

### Places

| `slot`          | Plays when                           |
| --------------- | ------------------------------------ |
| `tab.highlight` | The mark of the tab on show moves.   |
| `menu.enter`    | A menu opens.                        |
| `toast.enter`   | A notice appears.                    |
| `toast.exit`    | A notice leaves.                     |
| `palette.enter` | The palette opens.                   |
| `panel.enter`   | A panel opens.                       |
| `float.enter`   | A floating pane appears.             |
| `pill.enter`    | The pill of a pane put away appears. |
| `status.change` | A status item changes.               |

`tab.highlight` is described differently from the rest. See
[the tab's mark](#the-tabs-mark) below.

### Keyframes

An effect has from 2 to 8 keyframes. Each may set:

| Key          | Value                                                                                                               |
| ------------ | ------------------------------------------------------------------------------------------------------------------- |
| `offset`     | Where in the effect the keyframe is, from 0 to 1. Offsets never go down. Left out, the keyframes are spread evenly. |
| `opacity`    | From 0, unseen, to 1.                                                                                               |
| `transform`  | A list of the functions below, or `none`.                                                                           |
| `filter`     | `blur(...)`, `brightness(...)`, both, or `none`.                                                                    |
| `color`      | The text's colour, as one of the theme's colours: `token(color.accent)`.                                            |
| `background` | The background's colour, written the same way.                                                                      |

No other property can be set, and no colour can be written as a value: only as a token of the
theme.

What `transform` may hold:

| Function                          | Limit                                                               |
| --------------------------------- | ------------------------------------------------------------------- |
| `translate(x)`, `translate(x, y)` | Lengths in `px`, 200px at most either way, or in `%`, 100% at most. |
| `translateX(x)`, `translateY(y)`  | The same.                                                           |
| `scale(n)`, `scale(x, y)`         | From 0 to 4.                                                        |
| `scaleX(n)`, `scaleY(n)`          | From 0 to 4.                                                        |
| `rotate(a)`                       | An angle in `deg`, 360deg at most either way.                       |
| `skewX(a)`, `skewY(a)`            | An angle in `deg`, 45deg at most either way.                        |

What `filter` may hold:

| Function        | Limit             |
| --------------- | ----------------- |
| `blur(n)`       | From 0px to 20px. |
| `brightness(n)` | From 0 to 3.      |

### Where an effect ends

An effect that brings something in must end with the thing as it rests, so nothing jumps when the
effect is over. Its last keyframe has opacity 1 (or none), no transform that moves, scales or turns
it, no filter that changes it, and no colour.

`toast.exit` takes something away. It is the other way round: its first keyframe is the thing as it
rests.

An effect plays once and leaves nothing behind.

### Length and curve

| Key        | Value                                                       |
| ---------- | ----------------------------------------------------------- |
| `duration` | Milliseconds, from 1 to 1000. Needed.                       |
| `delay`    | Milliseconds before it starts: 500 at most. 0 if not given. |
| `easing`   | The curve. `standard` if not given.                         |

The curves:

| Written as                                             | What it is                                                                                                |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` | The named curves of CSS.                                                                                  |
| `standard`, `emphasized`, `exit`                       | The curves of the application's own motion.                                                               |
| `cubic-bezier(x1, y1, x2, y2)`                         | `x1` and `x2` are from 0 to 1. `y1` and `y2` are held to between -2 and 3.                                |
| `steps(n)`, `steps(n, position)`                       | From 1 to 60 steps. The position is `start`, `end`, `jump-start`, `jump-end`, `jump-none` or `jump-both`. |
| `spring(stiffness, damping)`                           | Stiffness from 50 to 2000, damping from 0.1 to 2.                                                         |

### The tab's mark

`tab.highlight` says how the mark of the tab on show moves from tab to tab. It takes a `highlight`
table and nothing else: no `keyframes`, `duration` or `easing` of its own.

```toml
[[contributes.motions]]
id = "snappy"
title = "Snappy"
slot = "tab.highlight"

[contributes.motions.highlight]
shape = "pill"
lead = [600.0, 0.8]
trail = [300.0, 0.9]
slide = "spring"

[contributes.motions.highlight.enter]
duration = 160
keyframes = [
  { opacity = 0.0, transform = "scale(0.9)" },
  { opacity = 1.0, transform = "none" },
]
```

| Key       | Value                                                                                   |
| --------- | --------------------------------------------------------------------------------------- |
| `shape`   | `pill`, `underline` or `glow`.                                                          |
| `lead`    | The spring of the end that leads: `[stiffness, damping]`.                               |
| `trail`   | The spring of the end that trails.                                                      |
| `slide`   | How tabs slide to new places: `"spring"`, or `{ duration = 200, easing = "ease-out" }`. |
| `enter`   | How a new tab comes in: `keyframes`, with either a `duration` or a `spring`.            |
| `renamed` | Keyframes for how a renamed label settles. Nothing if not given.                        |

A spring's stiffness is from 50 to 2000 and its damping from 0.1 to 2. A `slide` with a duration
takes any curve but a spring. The keyframes of `enter` and `renamed` follow the rules above and end
with the thing as it rests.

### How the user chooses a motion

An installed motion is offered in the settings, under the title its manifest gives it. A motion for
`tab.highlight` joins the choices of the tab highlight setting. Every other place a plugin offers
something for gets a row of its own in the Appearance settings, where Lume's own motion stays one
of the choices.

## Cursor effects

A cursor effect is drawn by a shader the plugin ships.

```toml
[[contributes.cursor-effects]]
id = "glow"
title = "Glow"
kind = "pulse"
shader = "shaders/glow.glsl"
duration = 300
reach = 32
```

| Key        | Value                                                                                                   |
| ---------- | ------------------------------------------------------------------------------------------------------- |
| `kind`     | `trail`, drawn when the cursor jumps, or `pulse`, drawn when it changes shape or lands in another pane. |
| `shader`   | A `.glsl` file in the package.                                                                          |
| `duration` | How long the effect lasts, in milliseconds: from 1 to 1500. 400 if not given.                           |
| `reach`    | How far from the cursor's cells it may draw, in pixels: 256 at most. 48 if not given.                   |

An installed effect joins the choices of the cursor trail or the cursor pulse setting, by its
`kind`, under its `title`. The user's setting for how long effects last is applied on top of
`duration`.

### The shader

The file is GLSL ES 3.00. It defines one function, with any helpers of its own:

```glsl
vec4 lume_effect(vec2 pixel)
```

Lume puts the file inside a program of its own, which declares everything the function can read
and calls it for each pixel near the cursor.

- `pixel` is the pixel being shaded, in device pixels, counted from the top left of the window's
  drawing area: x to the right, y down.
- The function returns a colour and how much of it covers the pixel: `vec4(red, green, blue,
cover)`. Do not premultiply. Lume clamps all four to between 0 and 1 and premultiplies itself.
  A cover of 0 or less draws nothing.

What the function can read:

| Name              | Type    | What it is                                                            |
| ----------------- | ------- | --------------------------------------------------------------------- |
| `lume_resolution` | `vec2`  | The size of the drawing area, in device pixels.                       |
| `lume_progress`   | `float` | How far the effect has got, from 0 to 1.                              |
| `lume_time`       | `float` | Seconds since the effect began.                                       |
| `lume_from`       | `vec4`  | The cell the cursor left: left, top, width, height, in device pixels. |
| `lume_to`         | `vec4`  | The cell the cursor reached, written the same way.                    |
| `lume_color`      | `vec3`  | The cursor's colour, each part from 0 to 1.                           |

The function can read nothing else. There is no texture to sample, so a shader never sees what a
terminal shows, and what it draws goes only to the screen.

Only the pixels near the effect are shaded: the rectangle round both cells, out as far as `reach`.
Whatever the function returns for a pixel further out is never drawn. For a trail, the cell the
cursor reached is left clear, so the cursor itself is never painted over.

A full example, a glow round the cell the cursor lands in:

```glsl
// A glow round the cell the cursor lands in.
float box(vec2 p, vec4 r) {
  vec2 d = abs(p - (r.xy + r.zw * 0.5)) - r.zw * 0.5;
  return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

vec4 lume_effect(vec2 pixel) {
  float d = box(pixel, lume_to);
  float glow = 0.0;
  for (int i = 0; i < 4; i++) {
    glow += exp(-max(d, 0.0) / (4.0 + float(i) * 3.0)) * 0.25;
  }
  return vec4(lume_color, glow * (1.0 - lume_progress));
}
```

### The rules a shader must keep

A shader is checked before any window compiles it. The checks keep it from reaching past its
function and hold down what it costs to run.

| Rule                                                                                                                                            | Why                                       |
| ----------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| 16 KiB at most, plain ASCII text.                                                                                                               | It is short.                              |
| It defines `vec4 lume_effect(vec2 ...)`.                                                                                                        | That is what Lume calls.                  |
| No preprocessor: no line may use `#`.                                                                                                           | Lume writes the program's head.           |
| No `uniform`, `attribute`, `varying`, `layout`, `buffer`, `shared`.                                                                             | It has no inputs or outputs of its own.   |
| No `in`, `out` or `inout` outside a function's parameters.                                                                                      | The same.                                 |
| No `precision`, `invariant`, `centroid`, `flat`, `smooth`.                                                                                      | Lume decides how the program is built.    |
| No `texture`, `texelFetch`, `textureLod`, `textureSize`, `textureGrad`, `textureProj`, `textureOffset`, and no name that begins with `sampler`. | There is nothing to sample.               |
| No `dFdx`, `dFdy`.                                                                                                                              | Not offered.                              |
| No name that begins with `gl_` or `__`.                                                                                                         | Those are the shader language's own.      |
| No name that begins with `lume_`, other than `lume_effect` and the six names in the table above.                                                | Those are the application's.              |
| No `while`, `do` or `goto`.                                                                                                                     | Every loop must have a bound.             |
| Braces are balanced.                                                                                                                            | The function cannot close Lume's program. |

These words are refused wherever they stand, as the name of a variable too. `smoothstep` is fine;
a variable called `smooth` or `texture` is not.

Loops have rules of their own:

- A loop is a `for` over an `int` counter: `for (int i = 0; i < 8; i++) { ... }`.
- The start and the bound are whole numbers written out, not variables or expressions. They may be
  negative.
- The test is `<`, `<=`, `>` or `>=` against the counter.
- The step is `i++`, `i--`, `i += N` or `i -= N`, and must go towards the bound.
- The body is in braces.
- One loop runs 64 times at most.
- Loops inside each other run 1024 times at most together: 32 by 32 is the most for two.

A shader that breaks a rule stops the install, and the message says which rule.

### When the effect plays

Cursor effects from plugins need WebGL2. Where the window cannot have it and draws its own effects
another way, a plugin's effect is not drawn.

The shader is fetched and built when the user chooses the effect, not when the cursor first jumps.
Where the browser engine can build shaders in the background, it does, and no frame waits for it.
The effect plays once the shader is ready. Cursor jumps before that go without it.

A shader that passes the checks but that the graphics driver cannot build is given up and not
tried again.
