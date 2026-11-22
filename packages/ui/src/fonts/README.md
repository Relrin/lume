# Bundled fonts

Two fonts are shipped with the application, so that it looks the same on every machine
whether or not they are installed there.

**Inter** is the font of the interface unless the configuration names another
(`font.ui-family`): tabs, headers, menus, settings, and the views of plugins.

- Font: Inter 4.1, the variable upright and italic faces (every weight in one file each)
- Source: https://github.com/rsms/inter at the tag `v4.1` (commit
  `e3a3d4c57d5ecc01453a575621882a384c1995a3`), `docs/font-files/InterVariable*.woff2`
- Licence: SIL Open Font License 1.1, in `OFL-Inter.txt`

**JetBrains Mono** is the font terminals are drawn in unless the configuration names
another (`font.family`).

- Font: JetBrains Mono 2.305, the regular, bold, italic and bold italic faces
- Source: https://github.com/JetBrains/JetBrainsMono at commit
  `02bb50b082dad9ef8a0f33ac393839202b760223`, `fonts/webfonts/JetBrainsMono-*.woff2`
- Licence: SIL Open Font License 1.1, in `OFL.txt`

`SHA256SUMS` has the SHA-256 of each file, which a test holds the files to.

`fonts.css` declares the faces. The page waits for the faces a terminal uses before it
draws the first one: glyphs are drawn into an atlas once, and an atlas made before the
font arrived would keep the fallback's glyphs. The application also serves these files
to the pages of plugins, which may load fonts from nowhere else but their own package.
