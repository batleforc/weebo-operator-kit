//! In-memory [`RemoteGateway`] for use-case and controller tests (exposed
//! to other crates by the `test-support` feature).

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::ports::{GatewayError, ObjectPatch, Remote, RemoteGateway, RemoteObject};

/// Remote objects keyed by name. Ids are handed out in creation order
/// (`"1"`, `"2"`…) and kept across updates, like a real server's. Every
/// write is recorded ([`FakeGateway::writes`]): a reconcile with nothing
/// to change must leave it empty.
#[derive(Default)]
pub struct FakeGateway {
    objects: Mutex<BTreeMap<String, (String, RemoteObject)>>,
    next_id: Mutex<u64>,
    failure: Mutex<Option<GatewayError>>,
    writes: Mutex<Vec<String>>,
}

impl FakeGateway {
    /// Every call fails with `failure` until it is set back to `None`.
    pub fn fail_with(&self, failure: Option<GatewayError>) {
        *self.failure.lock().unwrap() = failure;
    }

    pub fn get(&self, name: &str) -> Option<RemoteObject> {
        self.objects
            .lock()
            .unwrap()
            .get(name)
            .map(|(_, o)| o.clone())
    }

    /// Changes the object `name` behind the operator's back (a drift).
    pub fn edit(&self, name: &str, edit: impl FnOnce(&mut RemoteObject)) {
        if let Some((_, object)) = self.objects.lock().unwrap().get_mut(name) {
            edit(object);
        }
    }

    /// The writes received, oldest first: `create docs`,
    /// `update docs [description]`, `delete docs`.
    pub fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }

    pub fn clear_writes(&self) {
        self.writes.lock().unwrap().clear();
    }

    fn check(&self) -> Result<(), GatewayError> {
        match self.failure.lock().unwrap().clone() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn wrote(&self, write: String) {
        self.writes.lock().unwrap().push(write);
    }
}

#[async_trait::async_trait]
impl RemoteGateway for FakeGateway {
    async fn ping(&self) -> Result<(), GatewayError> {
        self.check()
    }

    async fn read(&self, name: &str) -> Result<Option<Remote>, GatewayError> {
        self.check()?;
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(name)
            .map(|(id, object)| Remote {
                id: id.clone(),
                object: object.clone(),
            }))
    }

    async fn create(&self, object: &RemoteObject) -> Result<String, GatewayError> {
        self.check()?;
        let id = {
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            next.to_string()
        };
        self.objects
            .lock()
            .unwrap()
            .insert(object.name.clone(), (id.clone(), object.clone()));
        self.wrote(format!("create {}", object.name));
        Ok(id)
    }

    async fn update(&self, name: &str, patch: &ObjectPatch) -> Result<(), GatewayError> {
        self.check()?;
        let mut objects = self.objects.lock().unwrap();
        let (_, object) = objects
            .get_mut(name)
            .ok_or_else(|| GatewayError::NotFound(name.to_string()))?;
        let mut fields = Vec::new();
        if let Some(description) = &patch.description {
            object.description = Some(description.clone());
            fields.push("description");
        }
        drop(objects);
        self.wrote(format!("update {name} [{}]", fields.join(", ")));
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), GatewayError> {
        self.check()?;
        match self.objects.lock().unwrap().remove(name) {
            Some(_) => {
                self.wrote(format!("delete {name}"));
                Ok(())
            }
            None => Err(GatewayError::NotFound(name.to_string())),
        }
    }
}
