#!/usr/bin/env node
// Downloads the CRD IDE's catalog (crd-catalog.config.mjs) into
// crd-catalog/<name>/: one YAML file per CRD, plus an operator's
// links.json when its tag has one. Run by `task site:catalog`, by hand,
// after changing a pin; the result is committed.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as yaml from 'js-yaml';
import catalog from '../crd-catalog.config.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'crd-catalog');

async function get(url) {
  const res = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  if (!res.ok) throw new Error(`${url}: ${res.status} ${res.statusText}`);
  return res.text();
}

function write(dir, file, content) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, file), content);
}

const crdFile = (crd) => `${crd.spec.names.singular ?? crd.spec.names.kind.toLowerCase()}.yaml`;

fs.rmSync(out, { recursive: true, force: true });

for (const { name, repo, tag } of catalog.operators) {
  // The directory listing, then each file: the tag's deploy/crd/ as is.
  const listing = JSON.parse(await get(`https://api.github.com/repos/${repo}/contents/deploy/crd?ref=${tag}`));
  const files = listing.filter((f) => f.type === 'file' && (f.name.endsWith('.yaml') || f.name === 'links.json'));
  for (const f of files) write(path.join(out, name), f.name, await get(f.download_url));
  console.log(`${name} ${tag}: ${files.length} files`);
}

for (const { name, url, kinds } of catalog.thirdParty) {
  const crds = yaml
    .loadAll(await get(url))
    .filter((d) => d?.kind === 'CustomResourceDefinition' && kinds.includes(d.spec.names.kind));
  const missing = kinds.filter((k) => !crds.some((c) => c.spec.names.kind === k));
  if (missing.length) throw new Error(`${name}: no CRD for ${missing.join(', ')} in ${url}`);
  for (const crd of crds) write(path.join(out, name), crdFile(crd), yaml.dump(crd, { lineWidth: -1, noRefs: true }));
  console.log(`${name}: ${crds.length} CRDs`);
}
