"""Writes the authored corpus recordings: `input.bin` and `script.toml` for each.

These five streams are authored here, not captured from real programs. They exercise the
dump format and the replay helper. Captures of real shells and full-screen programs are
added once Lume has a PTY to record from.

Run from the repository root:

    python -I tests/corpus/make_synthetic.py

Then regenerate the goldens and read them before keeping them:

    UPDATE_GOLDEN=1 cargo test -p lume-vt --test corpus
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parent
ESC = "\x1b"
CSI = ESC + "["


def write(name: str, description: str, cols: int, rows: int, data: str, resizes=()):
    """`resizes` is a list of (marker_index, cols, rows); offsets are byte offsets."""
    directory = ROOT / name
    directory.mkdir(exist_ok=True)
    payload = data.encode("utf-8")
    (directory / "input.bin").write_bytes(payload)
    lines = [
        f'description = "{description}"',
        f"cols = {cols}",
        f"rows = {rows}",
    ]
    for at, c, r in resizes:
        lines += ["", "[[resize]]", f"at = {at}", f"cols = {c}", f"rows = {r}"]
    (directory / "script.toml").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{name}: {len(payload)} bytes")


def plain_shell() -> str:
    out = []
    prompt = "user@host:~$ "
    # Enough output to scroll the 24-row screen. It comes first so that the cases after it
    # are still on screen when the dump is taken.
    out.append(prompt + "seq 1 30\r\n")
    for i in range(1, 31):
        out.append(f"{i}\r\n")
    out.append(prompt + "echo hello\r\nhello\r\n")
    # A typo corrected with backspace: the terminal sees BS, space, BS.
    out.append(prompt + "ls -lx" + "\b \b" + "a\r\n")
    out.append("total 12\r\n")
    for name in ["Cargo.toml", "README.md", "src"]:
        out.append(f"-rw-r--r--  1 user user  512 Nov 14 01:00 {name}\r\n")
    # Tabs advance to the next multiple of eight.
    out.append(prompt + "printf 'a\\tb\\tc\\n'\r\n")
    out.append("a\tb\tc\r\n")
    # A line longer than the screen is wide soft-wraps.
    out.append(prompt + "echo " + "w" * 90 + "\r\n")
    out.append("w" * 90 + "\r\n")
    # Carriage return alone returns to column 0 and overwrites.
    out.append("progress 10%\rprogress 100%\r\n")
    out.append(prompt)
    return "".join(out)


def colors_and_attributes() -> str:
    out = []
    sgr = lambda *codes: CSI + ";".join(str(c) for c in codes) + "m"
    reset = sgr(0)
    out.append("normal " + sgr(1) + "bold" + reset + " " + sgr(2) + "faint" + reset + " ")
    out.append(sgr(3) + "italic" + reset + " " + sgr(7) + "inverse" + reset + " ")
    out.append(sgr(9) + "strike" + reset + "\r\n")
    out.append(sgr(4) + "underline" + reset + " " + CSI + "4:3m" + "curly" + reset + " ")
    out.append(sgr(21) + "double" + reset + "\r\n")
    # The eight standard foreground colours, then the eight bright ones.
    out.append("".join(sgr(30 + i) + "X" for i in range(8)) + reset + " ")
    out.append("".join(sgr(90 + i) + "X" for i in range(8)) + reset + "\r\n")
    # Background colours.
    out.append("".join(sgr(40 + i) + " " for i in range(8)) + reset + "\r\n")
    # 256-colour and direct colour, foreground and background.
    out.append(sgr(38, 5, 208) + "orange208" + reset + " " + sgr(48, 5, 27) + "blue27" + reset + "\r\n")
    out.append(sgr(38, 2, 255, 128, 0) + "rgb-fg" + reset + " ")
    out.append(sgr(48, 2, 16, 32, 64) + "rgb-bg" + reset + "\r\n")
    # Attributes combine, and a partial reset keeps the rest.
    out.append(sgr(1, 3, 31) + "bold-italic-red" + sgr(22) + " italic-red" + sgr(23, 39) + " plain\r\n")
    # A style carried across a newline, then reset.
    out.append(sgr(32) + "green starts here\r\nand continues" + reset + " until reset\r\n")
    return "".join(out)


def wide_and_emoji() -> str:
    out = []
    out.append("ascii then 中文 then ascii\r\n")  # CJK, two columns each
    out.append("日本語のテキスト\r\n")
    out.append("emoji: \U0001f600 \U0001f680 done\r\n")  # single-codepoint emoji
    out.append("zwj: \U0001f468‍\U0001f469‍\U0001f467 end\r\n")  # family, joined
    out.append("flag: \U0001f1ef\U0001f1f5 end\r\n")  # regional indicators
    out.append("combining: é ä ô end\r\n")  # base plus combining mark
    out.append("vs16: ❤️ end\r\n")  # heart with emoji presentation selector
    # A wide character that does not fit in the last column wraps whole.
    out.append("x" * 39 + "中" + "tail\r\n")
    out.append("half-width kana: ｶﾞｷﾞ end\r\n")
    return "".join(out)


def alternate_screen() -> str:
    out = []
    goto = lambda row, col: f"{CSI}{row};{col}H"
    out.append("before the full-screen program\r\n$ editor file.txt\r\n")
    out.append(CSI + "?1049h")  # enter the alternate screen, saving the cursor
    out.append(CSI + "2J" + CSI + "H")
    # A title bar in inverse video across the full width.
    out.append(CSI + "7m" + " file.txt".ljust(40) + CSI + "0m")
    # A box drawn with cursor addressing.
    out.append(goto(3, 5) + "┌" + "─" * 20 + "┐")
    for row in range(4, 8):
        out.append(goto(row, 5) + "│" + goto(row, 26) + "│")
    out.append(goto(8, 5) + "└" + "─" * 20 + "┘")
    out.append(goto(5, 7) + CSI + "1m" + "Hello, editor" + CSI + "0m")
    # A scroll region on rows 10 to 14, scrolled by writing six lines into it.
    out.append(CSI + "10;14r" + goto(10, 1))
    for i in range(1, 7):
        out.append(f"region line {i}\r\n")
    out.append(CSI + "r")  # reset the scroll region
    # Erase to end of line, and insert and delete characters.
    out.append(goto(16, 1) + "erase-from-here>>>>>>>>" + goto(16, 16) + CSI + "K")
    out.append(goto(17, 1) + "abcdef" + goto(17, 3) + CSI + "2@" + goto(18, 1) + "abcdef" + goto(18, 3) + CSI + "2P")
    # A status line on the last row, then park the cursor and hide it.
    out.append(goto(20, 1) + CSI + "7m" + "-- INSERT --".ljust(40) + CSI + "0m")
    out.append(goto(5, 20) + CSI + "?25l")
    return "".join(out)


def resize_and_reflow():
    first = "The quick brown fox jumps over the lazy dog. " * 3 + "\r\n" + "short line\r\n"
    second = "after narrowing: " + "n" * 30 + "\r\n"
    third = "after widening\r\n"
    data = first + second + third
    at_narrow = len(first.encode("utf-8"))
    at_wide = at_narrow + len(second.encode("utf-8"))
    return data, [(at_narrow, 20, 12), (at_wide, 60, 12)]


def main():
    write("01-plain-shell", "A shell session: prompts, backspace, tabs, scrolling, soft wrap, carriage return", 80, 24, plain_shell())
    write("02-colors-attributes", "SGR colours and attributes: 16, 256 and direct colour, bold, italic, underline styles", 80, 24, colors_and_attributes())
    write("03-wide-emoji", "Wide characters, emoji, joined sequences, combining marks, wide wrap at the margin", 40, 12, wide_and_emoji())
    write("04-alternate-screen", "A full-screen program: alternate screen, cursor addressing, scroll region, erase, insert and delete", 40, 20, alternate_screen())
    data, resizes = resize_and_reflow()
    write("05-resize-reflow", "Soft-wrapped text reflowed by narrowing and then widening the grid", 40, 12, data, resizes)


if __name__ == "__main__":
    main()
