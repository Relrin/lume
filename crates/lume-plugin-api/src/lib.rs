//! The API Lume offers plugins' code, defined once.
//!
//! Every call a plugin can make is a row of [`api!`] below: its name, the arguments it
//! takes in JavaScript, the type of what it sends and of what it gets back, the
//! permission it needs, and what it does. From that one table come:
//!
//! - [`Call`], what the runtime reads a call into, and which the application's
//!   dispatcher matches on, so a call with no handler does not build;
//! - the functions on the `lume` object, written into the prelude the runtime loads
//!   before a plugin's code;
//! - the TypeScript typings authors write against (`@lume/plugin-api`);
//! - the reference in the documentation.
//!
//! A few parts of the API are not plain calls (commands, timers, listening for events,
//! logging); they are written by hand, in the prelude and the typings' templates beside
//! this file, next to the generated parts.

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::Value;

pub mod generate;
mod ts;
pub mod types;

pub use types::{ApiError, ErrorCode};
use types::{
    CommandExecute, ContextSet, KvKey, KvSet, Nothing, SettingsGet, StatusClear, StatusSet,
    Subscribe,
};

/// The major version of the API, which a manifest's `api` names.
pub const API_MAJOR: u32 = 1;

/// One call of the API.
pub struct Method {
    /// Its name: the namespace and the function, as in `status.set`.
    pub name: &'static str,
    /// The arguments the function takes in JavaScript, in order. Each is a field of
    /// what the call sends.
    pub args: &'static [&'static str],
    /// The permission it needs, by the name a manifest writes; none for the plugin's
    /// own things.
    pub permission: Option<&'static str>,
    /// What it does.
    pub doc: &'static str,
    /// Whether the function is reached only through the hand-written part of the
    /// prelude, not as `lume.<name>`.
    pub internal: bool,
    pub params: fn(&mut SchemaGenerator) -> Schema,
    pub result: fn(&mut SchemaGenerator) -> Schema,
}

/// Why what a plugin's code sent is not a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// No call has the name.
    Unknown(String),
    /// The arguments are not what the call takes.
    Invalid { name: &'static str, message: String },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "there is no call `{name}`"),
            Self::Invalid { name, message } => write!(f, "{name}: {message}"),
        }
    }
}

fn schema_of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// Defines the calls. Each row is
/// `Variant = "name"(js, args): Params => Result [, needs "permission"] [, internal = true];`.
macro_rules! api {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident = $name:literal ( $($arg:ident),* ) : $params:ty => $result:ty
        $(, needs $permission:literal)? $(, internal = $internal:literal)? ;
    )*) => {
        /// A call a plugin's code made, read.
        #[derive(Clone, Debug, PartialEq)]
        pub enum Call {
            $($variant($params),)*
        }

        impl Call {
            /// Reads a call from its name and the JSON its code sent.
            pub fn decode(name: &str, args: Value) -> Result<Self, DecodeError> {
                match name {
                    $($name => serde_json::from_value::<$params>(args)
                        .map(Self::$variant)
                        .map_err(|error| DecodeError::Invalid {
                            name: $name,
                            message: error.to_string(),
                        }),)*
                    other => Err(DecodeError::Unknown(other.to_owned())),
                }
            }

            /// The call's name.
            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => $name,)*
                }
            }

            /// The permission the call needs, if any.
            pub fn permission(&self) -> Option<&'static str> {
                method(self.name()).and_then(|method| method.permission)
            }
        }

        /// Every call of the API.
        pub static METHODS: &[Method] = &[
            $(Method {
                name: $name,
                args: &[$(stringify!($arg)),*],
                permission: api!(@permission $($permission)?),
                doc: concat!($($doc, "\n",)* ""),
                internal: api!(@internal $($internal)?),
                params: schema_of::<$params>,
                result: schema_of::<$result>,
            },)*
        ];
    };
    (@permission) => { None };
    (@permission $permission:literal) => { Some($permission) };
    (@internal) => { false };
    (@internal $internal:literal) => { $internal };
}

api! {
    /// Shows something in one of the plugin's status items. Fields left out keep what
    /// the item showed before.
    StatusSet = "status.set"(id, value, pane): StatusSet => ();
    /// Takes back what a status item showed, so it shows what its manifest says.
    StatusClear = "status.clear"(id, pane): StatusClear => ();
    /// Runs a command: one of the plugin's own, by its name, or one of the
    /// application's that plugins may run. Its arguments are a JSON object.
    CommandsExecute = "commands.execute"(id, args): CommandExecute => ();
    /// The value the plugin stored under a key, or null.
    KvGet = "kv.get"(key): KvKey => Value;
    /// Stores a value under a key. What a plugin stores is its own, and all of it
    /// together may be 1 MiB.
    KvSet = "kv.set"(key, value): KvSet => ();
    /// Removes what is stored under a key.
    KvDelete = "kv.delete"(key): KvKey => ();
    /// The keys the plugin has stored values under.
    KvList = "kv.list"(): Nothing => Vec<String>;
    /// One of the plugin's settings, by its key, or all of them as an object when no
    /// key is given. A setting the user has not set has its default.
    SettingsGet = "settings.get"(key): SettingsGet => Value;
    /// Sets a condition the plugin's keys and menu items can be shown under.
    ContextSet = "context.set"(key, value): ContextSet => ();
    /// Starts hearing about a topic.
    EventsSubscribe = "events.subscribe"(topic): Subscribe => (), needs "workspace.read", internal = true;
}

/// The call of this name.
pub fn method(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|method| method.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_call_is_read_from_its_name_and_arguments() {
        let call = Call::decode(
            "status.set",
            json!({ "id": "cpu", "value": { "value": 0.5, "text": "50%" } }),
        )
        .unwrap();
        assert_eq!(call.name(), "status.set");
        assert_eq!(call.permission(), None);
        let Call::StatusSet(set) = call else {
            panic!("not a status.set");
        };
        assert_eq!(set.value.value, Some(0.5));

        assert_eq!(
            Call::decode("events.subscribe", json!({ "topic": "session" }))
                .unwrap()
                .permission(),
            Some("workspace.read")
        );
        assert_eq!(
            Call::decode("status.blink", json!({})),
            Err(DecodeError::Unknown("status.blink".to_owned()))
        );
        assert!(matches!(
            Call::decode(
                "status.set",
                json!({ "id": "cpu", "value": {}, "colour": 1 })
            ),
            Err(DecodeError::Invalid {
                name: "status.set",
                ..
            })
        ));
        assert!(Call::decode("kv.list", json!({})).is_ok());
    }

    #[test]
    fn every_call_says_what_it_does_and_its_arguments_are_its_fields() {
        let mut generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        for method in METHODS {
            assert!(!method.doc.trim().is_empty(), "{}", method.name);
            let reference = (method.params)(&mut generator);
            let name = reference
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|path| path.rsplit('/').next())
                .unwrap_or_default()
                .to_owned();
            let definitions = generator.definitions().clone();
            let fields = definitions
                .get(&name)
                .and_then(|schema| schema.get("properties"))
                .and_then(Value::as_object)
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for arg in method.args {
                assert!(
                    fields.contains(&(*arg).to_owned()),
                    "{}: {arg}",
                    method.name
                );
            }
        }
    }
}
