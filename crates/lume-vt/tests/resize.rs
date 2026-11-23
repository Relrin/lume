//! What a terminal that grows taller does with its scrollback.

use lume_vt::Terminal;

/// A terminal three rows tall that has printed six lines, so that three have scrolled
/// off, grown to six rows.
fn grown(pulls: Option<bool>) -> Vec<String> {
    let mut terminal = Terminal::new(10, 3).unwrap();
    if let Some(pulls) = pulls {
        terminal.set_resize_pulls_scrollback(pulls).unwrap();
    }
    terminal.write(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
    assert_eq!(terminal.screen_text().unwrap(), ["four", "five", "six"]);
    terminal.resize(10, 6).unwrap();
    terminal.screen_text().unwrap()
}

#[test]
fn a_terminal_that_grows_fills_its_new_rows_from_scrollback_unless_told_not_to() {
    let filled = ["one", "two", "three", "four", "five", "six"];
    assert_eq!(grown(None), filled);
    assert_eq!(grown(Some(true)), filled);
    // Told not to, it leaves what scrolled off where it is, and the new rows are empty.
    assert_eq!(grown(Some(false)), ["four", "five", "six", "", "", ""]);
}
