//! How a session that ran over the OpenSSH client ended.
//!
//! The client's exit status alone does not say. Any status but its two own (255 and,
//! on Windows, -1) is the status of the shell at the other end. For those two, the
//! last thing the client printed tells the cases apart, and since the client's
//! messages are not translated, they can be matched. The answer is shown to the user
//! and used for nothing else; when no message fits, it says only that the connection
//! failed.

use lume_proto::{DisconnectReason, Disconnected, ExitStatus};

use crate::tuning::{SSH_DETAIL_MAX_CHARS, SSH_TAIL_BYTES, SSH_TAIL_LINES};

/// The status the client exits with when it fails or the connection does.
const CLIENT_FAILED: u32 = 255;

/// What the Windows client exits with when the connection is closed under it.
const CLIENT_CLOSED: u32 = u32::MAX;

/// The end of a session's output, kept as it goes by.
#[derive(Debug, Default)]
pub(crate) struct Tail {
    bytes: Vec<u8>,
}

impl Tail {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Takes the next bytes of the output.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= SSH_TAIL_BYTES {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - SSH_TAIL_BYTES..]);
            return;
        }
        let excess = (self.bytes.len() + bytes.len()).saturating_sub(SSH_TAIL_BYTES);
        self.bytes.drain(..excess);
        self.bytes.extend_from_slice(bytes);
    }

    /// The last lines of text in what was kept, the last one first, without the
    /// control sequences that were printed around them.
    fn lines(&self) -> Vec<String> {
        let text = plain(&self.bytes);
        text.lines()
            .rev()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(SSH_TAIL_LINES)
            .map(str::to_owned)
            .collect()
    }
}

/// The text in terminal output: what is left when escape sequences and control
/// characters other than line breaks are taken out.
fn plain(bytes: &[u8]) -> String {
    let mut text = String::new();
    let mut chars = String::from_utf8_lossy(bytes)
        .chars()
        .collect::<Vec<_>>()
        .into_iter()
        .peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // A control sequence runs to its final character.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                // A string runs to a bell or a string terminator.
                Some(']' | 'P' | '_' | '^' | 'X') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' => text.push('\n'),
            c if c.is_control() => {}
            c => text.push(c),
        }
    }
    text
}

/// What one of the client's lines says about how the connection ended, if anything.
fn reason_in(line: &str) -> Option<DisconnectReason> {
    const SAYS: [(&[&str], DisconnectReason); 4] = [
        (&["Permission denied"], DisconnectReason::AuthFailed),
        (
            &[
                "Host key verification failed",
                "HOST IDENTIFICATION HAS CHANGED",
                "Host key for",
            ],
            DisconnectReason::HostKey,
        ),
        (
            &[
                "Connection refused",
                "Could not resolve hostname",
                "timed out",
                "No route to host",
                "Network is unreachable",
            ],
            DisconnectReason::Unreachable,
        ),
        (
            &[
                "closed by remote host",
                "client_loop:",
                "Timeout, server",
                "Broken pipe",
                "Connection reset",
            ],
            DisconnectReason::ConnectionLost,
        ),
    ];
    SAYS.iter()
        .find(|(words, _)| words.iter().any(|word| line.contains(word)))
        .map(|(_, reason)| *reason)
}

/// Whether a line is the client's own farewell: `Connection to <host> closed.`
fn is_farewell(line: &str) -> bool {
    line.starts_with("Connection to ") && line.ends_with(" closed.")
}

/// Tells how an SSH session ended from the client's exit status and the end of what
/// it printed.
pub(crate) fn classify(status: ExitStatus, tail: &Tail) -> Disconnected {
    let lines = tail.lines();
    let detail = lines
        .first()
        .map(|line| line.chars().take(SSH_DETAIL_MAX_CHARS).collect::<String>());
    let reason = match status {
        // Not one of the client's own: the shell at the other end ended with it.
        ExitStatus::Code(code) if code != CLIENT_FAILED && code != CLIENT_CLOSED => {
            DisconnectReason::Exited
        }
        ExitStatus::Code(code) => {
            // The latest line that says something decides: what the session printed
            // before the connection went is not about the connection.
            let said = lines.iter().find_map(|line| {
                reason_in(line).or_else(|| {
                    is_farewell(line).then_some(if code == CLIENT_CLOSED {
                        DisconnectReason::Closed
                    } else {
                        DisconnectReason::Exited
                    })
                })
            });
            said.unwrap_or(DisconnectReason::Failed)
        }
        // The client itself was ended.
        ExitStatus::Signal(_) | ExitStatus::Unknown => DisconnectReason::Failed,
    };
    Disconnected { reason, detail }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(status: ExitStatus, printed: &str) -> Disconnected {
        let mut tail = Tail::new();
        tail.push(printed.as_bytes());
        classify(status, &tail)
    }

    fn reason(code: u32, printed: &str) -> DisconnectReason {
        ended(ExitStatus::Code(code), printed).reason
    }

    #[test]
    fn a_status_that_is_not_the_clients_own_is_the_shells() {
        for code in [0, 1, 7, 130, 254] {
            assert_eq!(
                reason(code, "$ exit 7\r\nlogout\r\nConnection to box closed.\r\n"),
                DisconnectReason::Exited
            );
        }
        // Whatever the session printed last.
        assert_eq!(
            reason(1, "cat: x: Permission denied\r\n"),
            DisconnectReason::Exited
        );
    }

    #[test]
    fn the_clients_last_words_tell_its_own_statuses_apart() {
        let cases = [
            (
                "me@box: Permission denied (publickey,password).\r\n",
                DisconnectReason::AuthFailed,
            ),
            (
                "Host key for box has changed and you have requested strict checking.\r\n\
                 Host key verification failed.\r\n",
                DisconnectReason::HostKey,
            ),
            (
                "ssh: connect to host box port 22: Connection refused\r\n",
                DisconnectReason::Unreachable,
            ),
            (
                "ssh: Could not resolve hostname nowhere: No such host is known.\r\n",
                DisconnectReason::Unreachable,
            ),
            (
                "ssh: connect to host box port 22: Connection timed out\r\n",
                DisconnectReason::Unreachable,
            ),
            (
                "$ \r\nclient_loop: send disconnect: Unknown error\r\n",
                DisconnectReason::ConnectionLost,
            ),
            (
                "Connection to jump closed by remote host.\r\n\
                 client_loop: send disconnect: Unknown error\r\n",
                DisconnectReason::ConnectionLost,
            ),
            (
                "Timeout, server box not responding.\r\n",
                DisconnectReason::ConnectionLost,
            ),
            ("something nobody expected\r\n", DisconnectReason::Failed),
            ("", DisconnectReason::Failed),
        ];
        for (printed, expected) in cases {
            assert_eq!(reason(CLIENT_FAILED, printed), expected, "{printed:?}");
        }
    }

    #[test]
    fn a_farewell_is_the_shells_end_or_a_closed_connection_by_the_status() {
        let farewell = "$ exit 255\r\nConnection to box closed.\r\n";
        assert_eq!(reason(CLIENT_FAILED, farewell), DisconnectReason::Exited);
        assert_eq!(reason(CLIENT_CLOSED, farewell), DisconnectReason::Closed);
        assert_eq!(
            reason(
                CLIENT_CLOSED,
                "client_loop: send disconnect: Unknown error\r\n"
            ),
            DisconnectReason::ConnectionLost
        );
    }

    #[test]
    fn what_the_session_printed_before_the_end_does_not_decide() {
        // A refused file a few lines up is not a refused login.
        let printed = "cat: /etc/shadow: Permission denied\r\n$ \r\n$ \r\n$ \r\n\
                       client_loop: send disconnect: Unknown error\r\n";
        assert_eq!(
            reason(CLIENT_FAILED, printed),
            DisconnectReason::ConnectionLost
        );
    }

    #[test]
    fn the_last_line_is_passed_on_without_what_was_printed_around_it() {
        let outcome = ended(
            ExitStatus::Code(CLIENT_FAILED),
            "\u{1b}[?25l\u{1b}]0;ssh\u{7}me@box: Permission denied (publickey).\u{1b}[K\r\n\u{1b}[?25h",
        );
        assert_eq!(outcome.reason, DisconnectReason::AuthFailed);
        assert_eq!(
            outcome.detail.as_deref(),
            Some("me@box: Permission denied (publickey).")
        );
        let long = "x".repeat(SSH_DETAIL_MAX_CHARS * 2);
        let detail = ended(ExitStatus::Code(CLIENT_FAILED), &long)
            .detail
            .unwrap();
        assert_eq!(detail.chars().count(), SSH_DETAIL_MAX_CHARS);
    }

    #[test]
    fn only_the_end_of_the_output_is_kept() {
        let mut tail = Tail::new();
        tail.push(&vec![b'a'; SSH_TAIL_BYTES * 3]);
        assert_eq!(tail.bytes.len(), SSH_TAIL_BYTES);
        tail.push(b"\r\nlast words\r\n");
        assert_eq!(tail.bytes.len(), SSH_TAIL_BYTES);
        assert_eq!(tail.lines()[0], "last words");
        for _ in 0..SSH_TAIL_BYTES {
            tail.push(b"b");
        }
        assert_eq!(tail.bytes, vec![b'b'; SSH_TAIL_BYTES]);
    }

    #[test]
    fn a_client_that_was_ended_itself_failed() {
        for status in [ExitStatus::Signal(9), ExitStatus::Unknown] {
            assert_eq!(
                ended(status, "Connection to box closed.\r\n").reason,
                DisconnectReason::Failed
            );
        }
    }
}
