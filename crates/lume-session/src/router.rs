//! The backends of the application, by id.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use lume_proto::BackendId;

use crate::{Result, SessionBackend, SessionError};

/// Maps backend ids to backends. A view stores the id of its session's backend and asks
/// here for the backend itself.
#[derive(Default)]
pub struct Router {
    backends: RwLock<HashMap<BackendId, Arc<dyn SessionBackend>>>,
}

impl Router {
    /// The id of the backend for sessions on this machine.
    pub const LOCAL: &'static str = "local";

    pub fn new() -> Self {
        Self::default()
    }

    /// The id of the backend for sessions on this machine.
    pub fn local_id() -> BackendId {
        BackendId(Self::LOCAL.to_owned())
    }

    /// Adds a backend, replacing any that had the id.
    pub fn insert(&self, id: BackendId, backend: Arc<dyn SessionBackend>) {
        self.backends
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, backend);
    }

    pub fn get(&self, id: &BackendId) -> Result<Arc<dyn SessionBackend>> {
        self.backends
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::UnknownBackend(id.clone()))
    }

    /// The ids of every backend, in order.
    pub fn ids(&self) -> Vec<BackendId> {
        let mut ids: Vec<BackendId> = self
            .backends
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }
}
