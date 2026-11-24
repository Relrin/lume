//! Text that nobody but its receiver is to see.

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

/// A password, or anything else that opens something. It prints as nothing, so that no
/// record of a message holds it, and its memory is wiped when it is dropped.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The text itself. What is made of it is the caller's to wipe.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_text_on_the_wire_and_nothing_in_a_record() {
        let secret = Secret::new("hunter2".to_owned());
        assert_eq!(serde_json::to_string(&secret).unwrap(), r#""hunter2""#);
        let read: Secret = serde_json::from_str(r#""hunter2""#).unwrap();
        assert_eq!(read, secret);
        assert_eq!(read.expose(), "hunter2");
        assert_eq!(
            format!("{:?} {:#?}", Some(&read), read.clone()),
            "Some(Secret) Secret"
        );
    }
}
