#!/usr/bin/env node
// Turns crd-catalog/ into public/crd-schemas/<name>/ (index.json,
// links.json, a schema per kind): what the CRD IDE page loads. Offline;
// run before every dev server and build (package.json `predev`/`prebuild`).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { generateCrdDocs } from '@weebo/operator-docs/crd-docs';
import catalog from '../crd-catalog.config.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'public/crd-schemas');

fs.rmSync(out, { recursive: true, force: true });
const entries = [...catalog.operators.map(({ name }) => ({ name })), ...catalog.thirdParty];
for (const { name, links } of entries) {
  const count = generateCrdDocs(
    { crdDir: `crd-catalog/${name}`, schemaDir: `public/crd-schemas/${name}`, pages: false, links: links ?? {} },
    { root },
  );
  console.log(`crd-schemas/${name}: ${count} kinds`);
}
