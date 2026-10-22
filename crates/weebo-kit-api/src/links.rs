//! What a CRD's fields say about other objects, for tools that compose
//! manifests (the docs' CRD IDE): the schema alone says a field is an
//! object with a `name`, not which kind that name designates, nor that the
//! operator writes a `Secret` of that name.
//!
//! Three markers in a field's JSON schema:
//!
//! - `x-weebo-link` `{ "to": [{ "group", "kind" } | { "role" }] }`: the field
//!   names another object. The kit's reference types carry it
//!   ([`crate::InstanceRef`], [`crate::SecretKeyRef`],
//!   [`crate::LocalSecretKeyRef`]); a field of another type, such as an
//!   [`crate::ObjectRef`], gets it from the operator:
//!
//!   ```ignore
//!   #[schemars(extend("x-weebo-link" = { "to": [{ "group": "example.weebo.io", "kind": "ExampleObject" }] }))]
//!   pub parent_ref: ObjectRef,
//!   ```
//!
//!   On a list field, the marker applies to each item (`spec.memberRefs[]`).
//!
//! - `x-weebo-produces` `{ "kind", "name": [..], "template"?, "each"?, "namespace"?, "unless"? }`:
//!   the object (each item, in a list) makes an object of `kind`, named by
//!   the first of its `name` fields that is set (`.`: the item itself, a
//!   plain string), else by `template`: `{path}` relative to the item,
//!   `{/path}` from the object's root, `{/ref>path}` read on the object
//!   the reference field `ref` designates —
//!   `"{/spec.instanceRef>spec.url}/o/{/spec.slug}/"`, a name the operator
//!   computes, which a `name` field lets the user override. [`crate::SecretTarget`] carries it: one `Secret` per
//!   target, none for a Vault one. On a list field, each item makes one
//!   (`spec.hosts[]`). `each` is a path under the field, for one the
//!   schema leaves free-form:
//!
//!   ```ignore
//!   // One schematic `Upstream` per URL of `config.upstreams`.
//!   #[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.example.weebo.io", "kind": "Upstream" }, "each": "upstreams[]", "name": ["."] }))]
//!   pub config: serde_json::Value,
//!   ```
//! - `x-weebo-keys` `["token", ..]`: on the field holding producing
//!   objects, the keys the made objects hold — the operator knows them,
//!   the kit's types do not:
//!
//!   ```ignore
//!   #[schemars(extend("x-weebo-keys" = ["token"]))]
//!   pub secret_targets: Vec<SecretTarget>,
//!   ```
//!
//! The markers never reach the CRD: Kubernetes refuses unknown `x-`
//! extensions, and `CustomResourceExt::crd()` drops them (its schema goes
//! through `JSONSchemaProps`, which ignores unknown keys). [`of`]
//! generates the schema again and reads them.
//!
//! A **schematic** kind ([`schematic`]) is one the IDE draws but Kubernetes
//! never sees: a cluster and the ways to reach it, the upstream a registry
//! mirrors. The operator describes its `spec` as a Rust type, markers
//! included; the IDE edits and links it like a CRD's, and exports it apart
//! from the manifests. A production may make one: such objects of the same
//! name are one node, the one added by hand among them.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::InstanceKind;

/// A reference field's marker.
pub const MARKER: &str = "x-weebo-link";
/// A producing object's marker.
pub const PRODUCES: &str = "x-weebo-produces";
/// The keys of the objects a field's producing objects make.
pub const KEYS: &str = "x-weebo-keys";

/// A Kubernetes kind, as `apiVersion` names its group (`""`: the core
/// group).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GroupKind {
    #[serde(default)]
    pub group: String,
    pub kind: String,
}

impl GroupKind {
    pub fn new(group: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            group: group.into(),
            kind: kind.into(),
        }
    }
}

/// The operator's two instance kinds, which the kit's [`crate::InstanceRef`]
/// names by role.
#[derive(Clone, Debug)]
pub struct Roles {
    pub instance: GroupKind,
    pub local_instance: GroupKind,
}

/// One reference field of a kind.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Dotted path from the object's root (`spec.instanceRef`); `[]` steps
    /// into an array's items, `*` into a map's values.
    pub path: String,
    /// The kinds the field may designate.
    pub to: Vec<GroupKind>,
}

/// An item is skipped when its field `path` holds `equals`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Unless {
    pub path: String,
    pub equals: String,
}

/// Objects a kind makes: one per value `each` reaches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Production {
    pub kind: GroupKind,
    /// Dotted path of the producing objects (`spec.secretTargets[]`); the
    /// other paths are relative to each of them.
    pub each: String,
    /// The fields naming the made object; the first one set wins.
    pub name: Vec<String>,
    /// The name when no `name` field is set, computed (see the module's
    /// doc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// The field naming its namespace; unset: the producing resource's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// The keys the made object holds, when the operator declares them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unless: Option<Unless>,
}

/// What a kind's fields say about other objects.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct KindLinks {
    pub references: Vec<Link>,
    pub produces: Vec<Production>,
}

/// A marker that does not have its expected shape.
#[derive(Debug)]
pub struct LinkError {
    pub marker: &'static str,
    pub path: String,
    pub source: serde_json::Error,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid {} at {}: {}",
            self.marker, self.path, self.source
        )
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkMarker {
    to: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Target {
    Role { role: InstanceKind },
    Kind(GroupKind),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProducesMarker {
    kind: GroupKind,
    #[serde(default)]
    name: Vec<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    each: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    unless: Option<Unless>,
}

/// The reference fields and productions of `K` (a
/// `#[derive(CustomResource)]` root type), sorted by path, the instance
/// roles resolved with `roles`.
pub fn of<K: JsonSchema>(roles: &Roles) -> Result<KindLinks, LinkError> {
    // Subschemas inlined, as kube does: a field's own marker and its
    // type's land in the same object.
    let schema = SchemaSettings::openapi3()
        .with(|s| s.inline_subschemas = true)
        .into_generator()
        .into_root_schema_for::<K>();
    let mut links = KindLinks::default();
    walk(schema.as_value(), "", &[], roles, &mut links)?;
    links.references.sort_by(|a, b| a.path.cmp(&b.path));
    links.produces.sort_by(|a, b| a.each.cmp(&b.each));
    Ok(links)
}

/// Where a schematic kind's objects live.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Namespaced,
    Cluster,
}

/// A kind the IDE draws but Kubernetes never sees: its `spec` schema and
/// links, under a group of the operator's (`schematic.example.weebo.io`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Schematic {
    pub group: String,
    pub scope: Scope,
    /// The `spec` schema (OpenAPI v3, subschemas inlined, markers
    /// removed); the type's doc comment is its description.
    pub schema: Value,
    pub references: Vec<Link>,
    pub produces: Vec<Production>,
}

/// The schematic kind whose `spec` is `S`, its links read from `S`'s
/// markers as [`of`] does (paths from the object's root: `spec.…`).
pub fn schematic<S: JsonSchema>(
    group: &str,
    scope: Scope,
    roles: &Roles,
) -> Result<Schematic, LinkError> {
    let mut schema = SchemaSettings::openapi3()
        .with(|s| {
            s.inline_subschemas = true;
            s.meta_schema = None;
        })
        .into_generator()
        .into_root_schema_for::<S>()
        .to_value();
    let mut links = KindLinks::default();
    walk(&schema, "spec", &[], roles, &mut links)?;
    links.references.sort_by(|a, b| a.path.cmp(&b.path));
    links.produces.sort_by(|a, b| a.each.cmp(&b.each));
    strip_markers(&mut schema);
    if let Value::Object(root) = &mut schema {
        // The Rust type's name, not the kind's.
        root.remove("title");
    }
    Ok(Schematic {
        group: group.to_string(),
        scope,
        schema,
        references: links.references,
        produces: links.produces,
    })
}

fn strip_markers(node: &mut Value) {
    match node {
        Value::Object(map) => {
            map.retain(|key, _| !key.starts_with("x-weebo-"));
            map.values_mut().for_each(strip_markers);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_markers),
        _ => {}
    }
}

fn read<T: DeserializeOwned>(
    marker: &'static str,
    value: &Value,
    path: &str,
) -> Result<T, LinkError> {
    serde_json::from_value(value.clone()).map_err(|source| LinkError {
        marker,
        path: path.to_string(),
        source,
    })
}

/// `keys`: the `x-weebo-keys` of the field holding this node (itself, or
/// the list it is an item of).
fn walk(
    node: &Value,
    path: &str,
    keys: &[String],
    roles: &Roles,
    out: &mut KindLinks,
) -> Result<(), LinkError> {
    let Some(node) = node.as_object() else {
        return Ok(());
    };
    let own_keys: Vec<String> = match node.get(KEYS) {
        Some(value) => read(KEYS, value, path)?,
        None => keys.to_vec(),
    };
    if let Some(marker) = node.get(MARKER) {
        let marker: LinkMarker = read(MARKER, marker, path)?;
        let mut to: Vec<GroupKind> = Vec::new();
        let resolved = marker.to.into_iter().map(|t| match t {
            Target::Role {
                role: InstanceKind::Instance,
            } => roles.instance.clone(),
            Target::Role {
                role: InstanceKind::LocalInstance,
            } => roles.local_instance.clone(),
            Target::Kind(gk) => gk,
        });
        // An operator with a single instance kind gives both roles the same.
        for gk in resolved {
            if !to.contains(&gk) {
                to.push(gk);
            }
        }
        // On a list field (`#[schemars(extend(..))]` on a `Vec<ObjectRef>`),
        // the marker lands on the array: each item is a reference.
        let path = if node.contains_key("items") {
            format!("{path}[]")
        } else {
            path.to_string()
        };
        out.references.push(Link { path, to });
    }
    if let Some(marker) = node.get(PRODUCES) {
        let marker: ProducesMarker = read(PRODUCES, marker, path)?;
        if marker.name.is_empty() && marker.template.is_none() {
            return Err(LinkError {
                marker: PRODUCES,
                path: path.to_string(),
                source: serde::de::Error::custom("neither a name field nor a template"),
            });
        }
        // On a list field, as `x-weebo-link`: each item makes one.
        let each = match marker.each {
            Some(under) => format!("{path}.{under}"),
            None if node.contains_key("items") => format!("{path}[]"),
            None => path.to_string(),
        };
        out.produces.push(Production {
            kind: marker.kind,
            each,
            name: marker.name,
            template: marker.template,
            namespace: marker.namespace,
            key_names: own_keys.clone(),
            unless: marker.unless,
        });
    }
    if let Some(Value::Object(properties)) = node.get("properties") {
        for (name, child) in properties {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            walk(child, &child_path, &[], roles, out)?;
        }
    }
    if let Some(items) = node.get("items") {
        walk(items, &format!("{path}[]"), &own_keys, roles, out)?;
    }
    if let Some(values) = node.get("additionalProperties") {
        walk(values, &format!("{path}.*"), &own_keys, roles, out)?;
    }
    // An `Option<T>` or an enum keeps its variants here, on the same path.
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(Value::Array(variants)) = node.get(key) {
            for variant in variants {
                walk(variant, path, &own_keys, roles, out)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InstanceRef, LocalSecretKeyRef, ObjectRef, SecretKeyRef, SecretTarget};
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::JSONSchemaProps;
    use std::collections::BTreeMap;

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Root {
        spec: Spec,
    }

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Spec {
        instance_ref: Option<InstanceRef>,
        token: SecretKeyRef,
        keys: Vec<LocalSecretKeyRef>,
        by_name: BTreeMap<String, LocalSecretKeyRef>,
        #[schemars(extend("x-weebo-link" = { "to": [{ "group": "example.weebo.io", "kind": "ExampleObject" }] }))]
        parent_ref: ObjectRef,
        plain: ObjectRef,
        #[schemars(extend("x-weebo-keys" = ["token"]))]
        secret_targets: Vec<SecretTarget>,
        #[schemars(extend("x-weebo-link" = { "to": [{ "group": "example.weebo.io", "kind": "ExampleObject" }] }))]
        member_refs: Vec<ObjectRef>,
        one_target: Option<SecretTarget>,
    }

    fn roles() -> Roles {
        Roles {
            instance: GroupKind::new("example.weebo.io", "ExampleInstance"),
            local_instance: GroupKind::new("example.weebo.io", "ExampleLocalInstance"),
        }
    }

    fn link(path: &str, to: &[(&str, &str)]) -> Link {
        Link {
            path: path.to_string(),
            to: to.iter().map(|(g, k)| GroupKind::new(*g, *k)).collect(),
        }
    }

    fn secret_target(each: &str, keys: &[&str]) -> Production {
        Production {
            kind: GroupKind::new("", "Secret"),
            each: each.to_string(),
            name: vec!["name".to_string()],
            template: None,
            namespace: Some("namespace".to_string()),
            key_names: keys.iter().map(|k| k.to_string()).collect(),
            unless: Some(Unless {
                path: "backend".to_string(),
                equals: "vault".to_string(),
            }),
        }
    }

    #[test]
    fn finds_the_kits_references_and_the_operators_markers() {
        let secret = [("", "Secret")];
        let links = of::<Root>(&roles()).unwrap();
        assert_eq!(
            links.references,
            vec![
                link("spec.byName.*", &secret),
                link(
                    "spec.instanceRef",
                    &[
                        ("example.weebo.io", "ExampleInstance"),
                        ("example.weebo.io", "ExampleLocalInstance"),
                    ],
                ),
                link("spec.keys[]", &secret),
                link(
                    "spec.memberRefs[]",
                    &[("example.weebo.io", "ExampleObject")]
                ),
                link("spec.parentRef", &[("example.weebo.io", "ExampleObject")]),
                link("spec.token", &secret),
            ]
        );
    }

    #[test]
    fn secret_targets_produce_secrets_with_the_fields_keys() {
        assert_eq!(
            of::<Root>(&roles()).unwrap().produces,
            vec![
                secret_target("spec.oneTarget", &[]),
                secret_target("spec.secretTargets[]", &["token"]),
            ]
        );
    }

    #[derive(JsonSchema)]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Bad {
        #[schemars(extend("x-weebo-link" = { "kinds": ["Secret"] }))]
        field: String,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code, reason = "only its schema is used")]
    struct BadKeys {
        #[schemars(extend("x-weebo-keys" = "token"))]
        field: Vec<SecretTarget>,
    }

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Computed {
        #[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.example.weebo.io", "kind": "Issuer" }, "name": ["issuerUrl"], "template": "{/spec.instanceRef>spec.url}/o/{/spec.slug}/" }))]
        oauth2: Option<Override>,
    }

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Override {
        issuer_url: Option<String>,
    }

    #[derive(JsonSchema)]
    #[expect(dead_code, reason = "only its schema is used")]
    struct Nameless {
        #[schemars(extend("x-weebo-produces" = { "kind": { "kind": "Secret" } }))]
        field: String,
    }

    #[test]
    fn a_computed_name_with_an_override() {
        let links = of::<Computed>(&roles()).unwrap();
        assert_eq!(links.produces.len(), 1);
        assert_eq!(links.produces[0].each, "oauth2");
        assert_eq!(links.produces[0].name, vec!["issuerUrl".to_string()]);
        assert_eq!(
            links.produces[0].template.as_deref(),
            Some("{/spec.instanceRef>spec.url}/o/{/spec.slug}/")
        );
        let err = of::<Nameless>(&roles()).unwrap_err();
        assert_eq!((err.marker, err.path.as_str()), (PRODUCES, "field"));
    }

    #[test]
    fn a_malformed_marker_names_its_field() {
        let err = of::<Bad>(&roles()).unwrap_err();
        assert_eq!((err.marker, err.path.as_str()), (MARKER, "field"));
        let err = of::<BadKeys>(&roles()).unwrap_err();
        assert_eq!((err.marker, err.path.as_str()), (KEYS, "field"));
    }

    /// A cluster, its access methods (one schematic kind referencing
    /// another), and an upstream a free-form field produces.
    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "only its schema is used")]
    struct AccessMethodSpec {
        /// The API server's URL.
        url: String,
        #[schemars(extend("x-weebo-link" = { "to": [{ "group": "schematic.example.weebo.io", "kind": "Cluster" }] }))]
        cluster_ref: ObjectRef,
    }

    fn open_object(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "object", "x-kubernetes-preserve-unknown-fields": true })
    }

    #[derive(JsonSchema)]
    #[expect(dead_code, reason = "only its schema is used")]
    struct WithUpstreams {
        // As an operator types a free-form field: `schema_with` keeps the marker.
        #[schemars(schema_with = "open_object")]
        #[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.example.weebo.io", "kind": "Upstream" }, "each": "upstreams[]", "name": ["."] }))]
        config: serde_json::Value,
        #[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.example.weebo.io", "kind": "Host" }, "name": ["."] }))]
        hosts: Vec<String>,
    }

    #[test]
    fn a_schematic_kind_has_its_spec_schema_and_links_without_markers() {
        let kind = schematic::<AccessMethodSpec>(
            "schematic.example.weebo.io",
            Scope::Namespaced,
            &roles(),
        )
        .unwrap();
        assert_eq!(
            kind.references,
            vec![link(
                "spec.clusterRef",
                &[("schematic.example.weebo.io", "Cluster")]
            )]
        );
        assert!(kind.produces.is_empty());
        assert_eq!(kind.schema["properties"]["url"]["type"], "string");
        assert!(kind.schema.get("title").is_none());
        assert!(!kind.schema.to_string().contains("x-weebo"));
        let json = serde_json::to_value(&kind).unwrap();
        assert_eq!(json["scope"], "Namespaced");
    }

    #[test]
    fn each_produces_under_a_free_form_field_and_per_item_of_a_list() {
        let links = of::<WithUpstreams>(&roles()).unwrap();
        let made = |kind: &str, each: &str| Production {
            kind: GroupKind::new("schematic.example.weebo.io", kind),
            each: each.to_string(),
            name: vec![".".to_string()],
            template: None,
            namespace: None,
            key_names: vec![],
            unless: None,
        };
        assert_eq!(
            links.produces,
            vec![
                made("Upstream", "config.upstreams[]"),
                made("Host", "hosts[]")
            ]
        );
    }

    /// What `CustomResourceExt::crd()` does with the schema: the markers
    /// must not reach the CRD.
    #[test]
    fn the_markers_never_reach_the_crd() {
        let schema = SchemaSettings::openapi3()
            .with(|s| {
                s.inline_subschemas = true;
                s.meta_schema = None;
            })
            .into_generator()
            .into_root_schema_for::<Root>();
        let raw = schema.as_value().to_string();
        assert!(raw.contains(MARKER) && raw.contains(PRODUCES) && raw.contains(KEYS));
        let props: JSONSchemaProps = serde_json::from_value(schema.to_value()).unwrap();
        assert!(!serde_json::to_string(&props).unwrap().contains("x-weebo"));
    }
}
