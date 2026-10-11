#!/usr/bin/env node
// weebo-crd-docs [--config <file>]
//
// Generates an operator's CRD reference pages and form schemas from its CRD
// YAML (see src/crd-docs/generate.mjs). Reads `crd-docs.config.mjs` from the
// current directory (the operator's docs site) unless --config says
// otherwise; relative paths in the config resolve against the config file's
// directory. Never hand-edit the generated files: re-run this instead.

import path from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { generateCrdDocs } from "../src/crd-docs/generate.mjs";

const { values } = parseArgs({
  options: {
    config: { type: "string", short: "c", default: "crd-docs.config.mjs" },
    help: { type: "boolean", short: "h" },
  },
});

if (values.help) {
  console.log("usage: weebo-crd-docs [--config crd-docs.config.mjs]");
  process.exit(0);
}

const configPath = path.resolve(values.config);
const { default: config } = await import(pathToFileURL(configPath).href);
const count = generateCrdDocs(config, { root: path.dirname(configPath) });

console.log(`Generated CRD docs + schemas for ${count} CRDs.`);
