//! What a command gives back when it carries out a verb of the command line.
//!
//! A verb is run in steps. Each call of its command gives back what to print and,
//! if there is more to come, a value to be called again with; the last step gives the
//! status the command line ends with.

use serde_json::Value;

/// The most text one step may print to either stream.
pub const STEP_TEXT_MAX: usize = 256 * 1024;

/// The largest value a step may ask to be called again with, as JSON.
pub const STEP_NEXT_MAX: usize = 64 * 1024;

/// The most text that may be piped to a verb.
pub const STDIN_MAX: usize = 512 * 1024;

/// The most text sent to the client in one message. A control message has a limit, and
/// text can grow several times over when it is written as JSON.
pub const OUTPUT_PIECE_MAX: usize = 64 * 1024;

/// One step of a verb, as its command gave it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// What to print as the result.
    pub out: String,
    /// What to say beside the result.
    pub err: String,
    /// What to call the command with again. Without it this was the last step.
    pub next: Option<Value>,
    /// The status to end with, which only the last step's counts.
    pub code: u8,
}

/// Reads a step from what a command gave back: nothing at all, or an object with any of
/// `out`, `err`, `next` and `code`.
pub fn read_step(value: Value) -> Result<Step, String> {
    let mut fields = match value {
        Value::Null => return Ok(Step::default()),
        Value::Object(fields) => fields,
        other => {
            return Err(format!(
                "gave back {}, where a verb's command gives an object with out, err, next \
                 and code, or nothing",
                kind(&other)
            ));
        }
    };
    let mut text = |name: &str| match fields.remove(name) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) if text.len() <= STEP_TEXT_MAX => Ok(text),
        Some(Value::String(_)) => Err(format!(
            "gave more than {} KiB as `{name}` in one step",
            STEP_TEXT_MAX / 1024
        )),
        Some(other) => Err(format!("gave {} as `{name}`, which is text", kind(&other))),
    };
    let out = text("out")?;
    let err = text("err")?;
    let next = match fields.remove("next") {
        None | Some(Value::Null) => None,
        Some(next) => {
            let size = serde_json::to_vec(&next).map_or(usize::MAX, |json| json.len());
            if size > STEP_NEXT_MAX {
                return Err(format!(
                    "gave more than {} KiB as `next`",
                    STEP_NEXT_MAX / 1024
                ));
            }
            Some(next)
        }
    };
    let code = match fields.remove("code") {
        None | Some(Value::Null) => 0,
        Some(code) => code
            .as_u64()
            .and_then(|code| u8::try_from(code).ok())
            .ok_or_else(|| {
                format!("gave {code} as `code`, which is a whole number from 0 to 255")
            })?,
    };
    Ok(Step {
        out,
        err,
        next,
        code,
    })
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Cuts text into pieces of at most `most` bytes, never inside a character.
pub fn pieces(text: &str, most: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.len() > most {
        let mut at = most;
        while !rest.is_char_boundary(at) {
            at -= 1;
        }
        let (piece, after) = rest.split_at(at);
        pieces.push(piece);
        rest = after;
    }
    if !rest.is_empty() {
        pieces.push(rest);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_step_is_read_from_what_a_command_gives_back() {
        assert_eq!(read_step(Value::Null), Ok(Step::default()));
        assert_eq!(read_step(json!({})), Ok(Step::default()));
        assert_eq!(
            read_step(json!({ "out": "a\n", "err": "b", "next": { "run": 4 }, "code": 9 })),
            Ok(Step {
                out: "a\n".to_owned(),
                err: "b".to_owned(),
                next: Some(json!({ "run": 4 })),
                code: 9,
            })
        );
        // A `next` of any kind but nothing goes on; nothing ends it.
        assert_eq!(
            read_step(json!({ "next": 0 })).unwrap().next,
            Some(json!(0))
        );
        assert_eq!(
            read_step(json!({ "next": null, "out": null }))
                .unwrap()
                .next,
            None
        );
        // What it does not know is passed over.
        assert_eq!(read_step(json!({ "more": true })), Ok(Step::default()));
    }

    #[test]
    fn what_is_not_a_step_is_said() {
        let said = |value: Value| read_step(value).unwrap_err();
        assert!(said(json!("text")).contains("gave back text"));
        assert!(said(json!([1])).contains("gave back a list"));
        assert!(said(json!({ "out": 3 })).contains("a number as `out`"));
        assert!(said(json!({ "err": ["x"] })).contains("a list as `err`"));
        for code in [json!(256), json!(-1), json!(1.5), json!("1")] {
            assert!(said(json!({ "code": code })).contains("0 to 255"), "{code}");
        }
        let long = "x".repeat(STEP_TEXT_MAX + 1);
        assert!(said(json!({ "out": long })).contains("more than 256 KiB as `out`"));
        assert!(read_step(json!({ "err": "x".repeat(STEP_TEXT_MAX) })).is_ok());
        let large = "x".repeat(STEP_NEXT_MAX);
        assert!(said(json!({ "next": large })).contains("more than 64 KiB as `next`"));
    }

    #[test]
    fn text_is_cut_between_characters() {
        assert_eq!(pieces("", 4), Vec::<&str>::new());
        assert_eq!(pieces("abc", 4), ["abc"]);
        assert_eq!(pieces("abcdefghi", 4), ["abcd", "efgh", "i"]);
        // Each of these is three bytes.
        assert_eq!(pieces("€€€", 4), ["€", "€", "€"]);
        assert_eq!(pieces("a€b€", 4), ["a€", "b€"]);
    }
}
