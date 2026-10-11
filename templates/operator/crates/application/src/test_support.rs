//! In-memory [`RemoteGateway`] for use-case and controller tests (exposed
//! to other crates by the `test-support` feature).

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::ports::{GatewayError, RemoteGateway, RemoteObject};

/// Remote objects keyed by name. Ids are handed out in creation order
/// (`"1"`, `"2"`…) and kept across updates, like a real server's.
#[derive(Default)]
pub struct FakeGateway {
    objects: Mutex<BTreeMap<String, (String, RemoteObject)>>,
    next_id: Mutex<u64>,
    failure: Mutex<Option<GatewayError>>,
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

    fn check(&self) -> Result<(), GatewayError> {
        match self.failure.lock().unwrap().clone() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl RemoteGateway for FakeGateway {
    async fn ping(&self) -> Result<(), GatewayError> {
        self.check()
    }

    async fn ensure(&self, object: &RemoteObject) -> Result<String, GatewayError> {
        self.check()?;
        let mut objects = self.objects.lock().unwrap();
        let id = match objects.get(&object.name) {
            Some((id, _)) => id.clone(),
            None => {
                let mut next = self.next_id.lock().unwrap();
                *next += 1;
                next.to_string()
            }
        };
        objects.insert(object.name.clone(), (id.clone(), object.clone()));
        Ok(id)
    }

    async fn delete(&self, name: &str) -> Result<(), GatewayError> {
        self.check()?;
        match self.objects.lock().unwrap().remove(name) {
            Some(_) => Ok(()),
            None => Err(GatewayError::NotFound(name.to_string())),
        }
    }
}
