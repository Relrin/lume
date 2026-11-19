//! Secrets a plugin keeps: tokens and keys, in the system's keychain.
//!
//! Each plugin's secrets are kept under a service name of its own, `Lume plugin <id>`, so
//! one plugin cannot ask for another's. A secret is kept in the Windows Credential
//! Manager, the macOS keychain, or the Secret Service elsewhere (GNOME Keyring, KWallet).
//! Where there is no keychain the call fails as unavailable: a secret is never written
//! anywhere in plain text instead.
//!
//! Windows keeps at most 2560 bytes in one credential, so a larger secret is refused
//! there as too large.

use crate::scope::ServiceError;

/// The longest key.
pub const KEY_MAX: usize = 128;

/// The largest secret, in bytes.
pub const SECRET_MAX: usize = 8 * 1024;

/// Where a plugin's secrets are kept.
pub trait Keychain: Send + Sync {
    /// The secret kept under `key`, or `None` when there is none.
    fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, ServiceError>;
    /// Keeps `secret` under `key`, in place of any kept before.
    fn set(&self, plugin: &str, key: &str, secret: &str) -> Result<(), ServiceError>;
    /// Forgets the secret kept under `key`. Forgetting one that is not there is no error.
    fn delete(&self, plugin: &str, key: &str) -> Result<(), ServiceError>;
}

/// The system's keychain.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsKeychain;

/// The service name a plugin's secrets are kept under.
pub fn service_name(plugin: &str) -> String {
    format!("Lume plugin {plugin}")
}

/// Checks a key: 1 to [`KEY_MAX`] letters, digits, dots, underscores and dashes.
pub fn check_key(key: &str) -> Result<(), ServiceError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if key.is_empty() || key.len() > KEY_MAX || !key.chars().all(allowed) {
        return Err(ServiceError::invalid(format!(
            "`{key}` is not a key: 1 to {KEY_MAX} letters, digits, `.`, `_` or `-`"
        )));
    }
    Ok(())
}

/// Checks a secret's size: at most [`SECRET_MAX`] bytes.
pub fn check_secret(secret: &str) -> Result<(), ServiceError> {
    if secret.len() > SECRET_MAX {
        return Err(ServiceError::invalid(format!(
            "a secret is at most {SECRET_MAX} bytes"
        )));
    }
    Ok(())
}

/// One kept secret's place in a keychain.
trait Slot {
    fn get_secret(&self) -> keyring::Result<Vec<u8>>;
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()>;
    fn delete_credential(&self) -> keyring::Result<()>;
}

impl Slot for keyring::Entry {
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        keyring::Entry::get_secret(self)
    }

    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        keyring::Entry::set_secret(self, secret)
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        keyring::Entry::delete_credential(self)
    }
}

/// What the keychain said, for the plugin. Never the secret.
fn keychain_error(error: keyring::Error, key: &str) -> ServiceError {
    match error {
        keyring::Error::NoDefaultStore => {
            ServiceError::unavailable("there is no keychain on this system to keep secrets in")
        }
        keyring::Error::NoStorageAccess(_) => ServiceError::unavailable(
            "the keychain is locked, or refused to let Lume keep secrets in it",
        ),
        keyring::Error::TooLong(..) => ServiceError::too_large(format!(
            "the secret for `{key}` is larger than this system's keychain keeps"
        )),
        keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
            ServiceError::failed(format!("what is kept for `{key}` is not text"))
        }
        keyring::Error::Invalid(..) => {
            ServiceError::invalid(format!("the keychain refused `{key}` as a key"))
        }
        keyring::Error::Ambiguous(_) => ServiceError::failed(format!(
            "the keychain holds more than one secret for `{key}`"
        )),
        other => ServiceError::failed(format!("the keychain failed: {other}")),
    }
}

/// Reads a secret from the slot `open` gives for a plugin and key.
fn get_from<S: Slot>(
    open: impl FnOnce(&str, &str) -> keyring::Result<S>,
    plugin: &str,
    key: &str,
) -> Result<Option<String>, ServiceError> {
    check_key(key)?;
    let slot = open(&service_name(plugin), key).map_err(|error| keychain_error(error, key))?;
    match slot.get_secret() {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| ServiceError::failed(format!("what is kept for `{key}` is not text"))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(keychain_error(error, key)),
    }
}

/// Keeps a secret in the slot `open` gives for a plugin and key.
fn set_in<S: Slot>(
    open: impl FnOnce(&str, &str) -> keyring::Result<S>,
    plugin: &str,
    key: &str,
    secret: &str,
) -> Result<(), ServiceError> {
    check_key(key)?;
    check_secret(secret)?;
    let slot = open(&service_name(plugin), key).map_err(|error| keychain_error(error, key))?;
    slot.set_secret(secret.as_bytes())
        .map_err(|error| keychain_error(error, key))
}

/// Forgets the secret in the slot `open` gives for a plugin and key.
fn delete_from<S: Slot>(
    open: impl FnOnce(&str, &str) -> keyring::Result<S>,
    plugin: &str,
    key: &str,
) -> Result<(), ServiceError> {
    check_key(key)?;
    let slot = open(&service_name(plugin), key).map_err(|error| keychain_error(error, key))?;
    match slot.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(keychain_error(error, key)),
    }
}

impl Keychain for OsKeychain {
    fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, ServiceError> {
        get_from(keyring::Entry::new, plugin, key)
    }

    fn set(&self, plugin: &str, key: &str, secret: &str) -> Result<(), ServiceError> {
        set_in(keyring::Entry::new, plugin, key, secret)
    }

    fn delete(&self, plugin: &str, key: &str) -> Result<(), ServiceError> {
        delete_from(keyring::Entry::new, plugin, key)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use keyring_core::api::CredentialStoreApi;

    use super::*;
    use crate::ErrorKind;

    impl Slot for keyring_core::Entry {
        fn get_secret(&self) -> keyring::Result<Vec<u8>> {
            keyring_core::Entry::get_secret(self)
        }

        fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
            keyring_core::Entry::set_secret(self, secret)
        }

        fn delete_credential(&self) -> keyring::Result<()> {
            keyring_core::Entry::delete_credential(self)
        }
    }

    /// The system keychain's ways, over keyring's store kept in memory.
    struct Mock(Arc<keyring_core::mock::Store>);

    impl Keychain for Mock {
        fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, ServiceError> {
            get_from(
                |service, user| self.0.build(service, user, None),
                plugin,
                key,
            )
        }

        fn set(&self, plugin: &str, key: &str, secret: &str) -> Result<(), ServiceError> {
            set_in(
                |service, user| self.0.build(service, user, None),
                plugin,
                key,
                secret,
            )
        }

        fn delete(&self, plugin: &str, key: &str) -> Result<(), ServiceError> {
            delete_from(
                |service, user| self.0.build(service, user, None),
                plugin,
                key,
            )
        }
    }

    fn mock() -> Mock {
        Mock(keyring_core::mock::Store::new().unwrap())
    }

    #[test]
    fn a_secret_is_kept_read_and_forgotten() {
        let keychain = mock();
        assert_eq!(keychain.get("a.x", "token").unwrap(), None);
        keychain.set("a.x", "token", "s3cret").unwrap();
        assert_eq!(
            keychain.get("a.x", "token").unwrap().as_deref(),
            Some("s3cret")
        );
        keychain.set("a.x", "token", "newer").unwrap();
        assert_eq!(
            keychain.get("a.x", "token").unwrap().as_deref(),
            Some("newer")
        );
        keychain.delete("a.x", "token").unwrap();
        assert_eq!(keychain.get("a.x", "token").unwrap(), None);
        keychain.delete("a.x", "token").unwrap();
    }

    #[test]
    fn each_plugin_has_its_own_secrets() {
        let keychain = mock();
        keychain.set("a.x", "token", "mine").unwrap();
        assert_eq!(keychain.get("b.y", "token").unwrap(), None);
        keychain.delete("b.y", "token").unwrap();
        assert_eq!(
            keychain.get("a.x", "token").unwrap().as_deref(),
            Some("mine")
        );
        assert_eq!(service_name("a.x"), "Lume plugin a.x");
    }

    #[test]
    fn a_key_and_a_secret_are_checked_before_the_keychain_is_touched() {
        let keychain = mock();
        for key in ["", "has space", "slash/key", "ключ", &"k".repeat(129)] {
            assert_eq!(
                keychain.set("a.x", key, "x").unwrap_err().kind,
                ErrorKind::Invalid,
                "{key}"
            );
            // The system's keychain is not reached either: the key is refused first.
            assert_eq!(
                OsKeychain.get("a.x", key).unwrap_err().kind,
                ErrorKind::Invalid
            );
        }
        keychain.set("a.x", &"k".repeat(128), "x").unwrap();
        keychain
            .set("a.x", "a-b_c.d", &"x".repeat(SECRET_MAX))
            .unwrap();
        let large = keychain
            .set("a.x", "big", &"x".repeat(SECRET_MAX + 1))
            .unwrap_err();
        assert_eq!(large.kind, ErrorKind::Invalid);
        assert!(!large.message.contains("xxxx"));
    }

    #[test]
    fn a_keychain_that_is_not_there_is_unavailable() {
        let error = keychain_error(keyring::Error::NoDefaultStore, "token");
        assert_eq!(error.kind, ErrorKind::Unavailable);
        let locked = keychain_error(
            keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked"))),
            "token",
        );
        assert_eq!(locked.kind, ErrorKind::Unavailable);
        let refused = set_in(
            |_, _| -> keyring::Result<keyring_core::Entry> { Err(keyring::Error::NoDefaultStore) },
            "a.x",
            "token",
            "x",
        )
        .unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Unavailable);
    }
}
