//! The annotations every operator reads on its CRs, under its own domain
//! (`forgejo.weebo.io/force-orphan`, `authentik.weebo.io/force-orphan`…):
//! the same behaviour behind the same name in every operator.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

/// `"true"`: the finalizer is released without cleanup — the remote object
/// and the emitted secrets are left as they are. The way out for a CR whose
/// cleanup can't succeed any more (its instance already gone).
pub const FORCE_ORPHAN: &str = "force-orphan";
/// `"true"`: allows a change the operator can only apply by deleting and
/// recreating the remote object (losing what it held).
pub const ALLOW_DISRUPTIVE_UPDATE: &str = "allow-disruptive-update";
/// Any new value: re-issues the secret the CR emits, once per value.
pub const ROTATE_SECRET: &str = "rotate-secret";
/// Present: the importer is still seeding the CR's status; the controller
/// leaves it alone (a deletion is still processed).
pub const IMPORT_PENDING: &str = "import-pending";
/// The owner id the CR's emitted secrets were recorded under before it was
/// moved (set by a migration): they are still its own, and are rewritten
/// under its current id.
pub const PREVIOUS_OWNER: &str = "previous-owner";

/// The annotations of one operator, under `domain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annotations {
    pub domain: &'static str,
}

impl Annotations {
    pub const fn new(domain: &'static str) -> Self {
        Self { domain }
    }

    /// `<domain>/<name>`.
    pub fn key(&self, name: &str) -> String {
        format!("{}/{name}", self.domain)
    }

    pub fn get<'a>(&self, meta: &'a ObjectMeta, name: &str) -> Option<&'a str> {
        meta.annotations
            .as_ref()?
            .get(&self.key(name))
            .map(String::as_str)
    }

    fn is_true(&self, meta: &ObjectMeta, name: &str) -> bool {
        self.get(meta, name) == Some("true")
    }

    pub fn force_orphan(&self, meta: &ObjectMeta) -> bool {
        self.is_true(meta, FORCE_ORPHAN)
    }

    pub fn allows_disruptive_update(&self, meta: &ObjectMeta) -> bool {
        self.is_true(meta, ALLOW_DISRUPTIVE_UPDATE)
    }

    pub fn rotate_nonce(&self, meta: &ObjectMeta) -> Option<String> {
        self.get(meta, ROTATE_SECRET).map(str::to_string)
    }

    pub fn previous_owner<'a>(&self, meta: &'a ObjectMeta) -> Option<&'a str> {
        self.get(meta, PREVIOUS_OWNER)
    }

    /// Paused by the importer — never while the CR is being deleted.
    pub fn import_pending(&self, meta: &ObjectMeta) -> bool {
        meta.deletion_timestamp.is_none() && self.get(meta, IMPORT_PENDING).is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use k8s_openapi::jiff::Timestamp;

    use super::*;

    const A: Annotations = Annotations::new("example.weebo.io");

    fn meta(pairs: &[(&str, &str)]) -> ObjectMeta {
        ObjectMeta {
            annotations: Some(
                pairs
                    .iter()
                    .map(|(k, v)| (format!("example.weebo.io/{k}"), v.to_string()))
                    .collect::<BTreeMap<_, _>>(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn flags_need_true() {
        assert!(A.force_orphan(&meta(&[(FORCE_ORPHAN, "true")])));
        assert!(!A.force_orphan(&meta(&[(FORCE_ORPHAN, "yes")])));
        assert!(!A.force_orphan(&ObjectMeta::default()));
        assert!(A.allows_disruptive_update(&meta(&[(ALLOW_DISRUPTIVE_UPDATE, "true")])));
        assert_eq!(A.key(FORCE_ORPHAN), "example.weebo.io/force-orphan");
    }

    #[test]
    fn rotation_and_import() {
        assert_eq!(
            A.rotate_nonce(&meta(&[(ROTATE_SECRET, "2")])).as_deref(),
            Some("2")
        );
        let mut paused = meta(&[(IMPORT_PENDING, "true")]);
        assert!(A.import_pending(&paused));
        paused.deletion_timestamp = Some(Time(Timestamp::from_second(1).unwrap()));
        assert!(!A.import_pending(&paused), "a deletion is always processed");
    }
}
