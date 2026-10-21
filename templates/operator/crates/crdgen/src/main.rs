//! Writes `deploy/crd/<kind, lowercase>.yaml`, one file per CRD, from the
//! `api` types (`CustomResourceExt::crd()`), and `deploy/crd/links.json`,
//! the reference fields and produced objects of each kind
//! (`weebo_kit_api::links`) for the docs' CRD IDE. Run by `task recu`; the result is committed and installed
//! by the chart and the envtest tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::CustomResourceExt;
use schemars::JsonSchema;
use weebo_kit_api::links::{self, GroupKind, KindLinks, Roles};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "deploy/crd")]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    fs::create_dir_all(&args.out)?;
    let roles = Roles {
        instance: group_kind(&api::Instance::crd()),
        local_instance: group_kind(&api::LocalInstance::crd()),
    };
    let mut kinds = BTreeMap::new();
    write::<api::Instance>(&args.out, &roles, &mut kinds)?;
    write::<api::LocalInstance>(&args.out, &roles, &mut kinds)?;
    write::<api::Object>(&args.out, &roles, &mut kinds)?;
    let links = serde_json::json!({ "version": 1, "kinds": kinds });
    fs::write(
        args.out.join("links.json"),
        serde_json::to_string_pretty(&links)? + "\n",
    )?;
    println!("wrote CRDs and links.json to {}", args.out.display());
    Ok(())
}

fn write<K: CustomResourceExt + JsonSchema>(
    out: &Path,
    roles: &Roles,
    kinds: &mut BTreeMap<String, KindLinks>,
) -> anyhow::Result<()> {
    let crd = K::crd();
    let kind = crd.spec.names.kind.clone();
    fs::write(
        out.join(format!("{}.yaml", kind.to_lowercase())),
        serde_norway::to_string(&crd)?,
    )?;
    kinds.insert(kind, links::of::<K>(roles)?);
    Ok(())
}

fn group_kind(crd: &CustomResourceDefinition) -> GroupKind {
    GroupKind::new(&crd.spec.group, &crd.spec.names.kind)
}
