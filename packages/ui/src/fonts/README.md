# Bundled fonts

JetBrains Mono is the font terminals are drawn in unless the configuration names another
(`font.family`). It is shipped with the application so that the default looks the same on
every machine, whether or not the font is installed there.

- Font: JetBrains Mono 2.305, the regular, bold, italic and bold italic faces
- Source: https://github.com/JetBrains/JetBrainsMono at commit
  `02bb50b082dad9ef8a0f33ac393839202b760223`, `fonts/webfonts/JetBrainsMono-*.woff2`
- Licence: SIL Open Font License 1.1, in `OFL.txt`
- SHA-256 of each file: `SHA256SUMS`, which a test holds the files to

`fonts.css` declares the four faces. The page waits for the faces a terminal uses before
it draws the first one: glyphs are drawn into an atlas once, and an atlas made before the
font arrived would keep the fallback's glyphs.
