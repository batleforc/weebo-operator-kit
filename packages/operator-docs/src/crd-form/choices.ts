// Guided choices for the manifest builder, in the spirit of a config
// generator: several kinds accept mutually exclusive ways of doing one thing
// (credentials go to a Kubernetes Secret or to Vault, a resource is created
// empty or from a template). The schema can only say "these are all
// optional"; the operator's choice table says which go together, so the form
// asks "how?" first and shows only the matching fields. This file is the
// machinery; the table itself is per operator (`CrdChoices`).
//
// Field names are `spec` keys. A kind not listed gets the plain form.

import { isRecord, type FieldSchema } from "./types";

export interface ChoiceOption {
  id: string;
  label: string;
  /** One line under the segmented control, shown while this option is chosen. */
  hint: string;
  /** Spec fields shown (and kept in the manifest) only for this option. */
  fields: string[];
  /** Of those, the ones this option can't do without (marked required).
   * A dotted path (`tls.caSecretRef`) marks a nested field required too. */
  require?: string[];
  /** Spec values this option implies; their fields are never shown. */
  set?: Record<string, unknown>;
  /** Form values written in when this option is picked; unlike `set`, the
   * fields stay visible and editable (e.g. Vault preselects `backend`). */
  preset?: Record<string, unknown>;
}

export interface ChoiceGroup {
  id: string;
  label: string;
  options: ChoiceOption[];
}

/** Every kind's choice groups, keyed by kind. Declared by the operator (its
 * `crd-choices.ts`), handed to `CrdForm`/`CrdBuilder` through `docsMdxComponents`. */
export type CrdChoices = Record<string, ChoiceGroup[]>;

export type Selection = Record<string, string>;

export function defaultSelection(groups: ChoiceGroup[]): Selection {
  return Object.fromEntries(groups.flatMap((g) => (g.options[0] ? [[g.id, g.options[0].id]] : [])));
}

function chosen(groups: ChoiceGroup[], selection: Selection): ChoiceOption[] {
  return groups.flatMap((g) => g.options.find((o) => o.id === selection[g.id]) ?? g.options.slice(0, 1));
}

/** Spec keys the form must not render: other options' fields, and implied values. */
export function hiddenKeys(groups: ChoiceGroup[], selection: Selection): string[] {
  const picked = chosen(groups, selection);
  const shown = new Set(picked.flatMap((o) => o.fields));
  const hidden = new Set<string>();
  for (const group of groups) {
    for (const option of group.options) {
      for (const key of option.fields) if (!shown.has(key)) hidden.add(key);
      for (const key of Object.keys(option.set ?? {})) hidden.add(key);
    }
  }
  return [...hidden];
}

/** The spec as the manifest should carry it: hidden fields dropped, implied values set. */
export function applySelection(
  groups: ChoiceGroup[],
  selection: Selection,
  spec: Record<string, unknown>,
): Record<string, unknown> {
  const out = { ...spec };
  for (const key of hiddenKeys(groups, selection)) delete out[key];
  for (const option of chosen(groups, selection)) Object.assign(out, option.set ?? {});
  return out;
}

/** The schema with the chosen options' must-have fields marked required. */
export function withChoiceRequirements<S extends FieldSchema>(
  groups: ChoiceGroup[],
  selection: Selection,
  schema: S,
): S {
  const extra = chosen(groups, selection).flatMap((o) => o.require ?? []);
  return extra.reduce((acc, path) => requirePath(acc, path.split(".")), schema);
}

// Copy-on-write down `path`, adding its last segment to its parent's `required`.
function requirePath<S extends FieldSchema>(schema: S, path: string[]): S {
  const [head, ...rest] = path;
  if (head === undefined) return schema;
  if (rest.length === 0) {
    return { ...schema, required: [...new Set([...(schema.required ?? []), head])] };
  }
  const child = schema.properties?.[head];
  if (!child) return schema;
  return { ...schema, properties: { ...schema.properties, [head]: requirePath(child, rest) } };
}

/** Deep-merge `preset` into a form value: objects merge, anything else replaces. */
export function withPreset(value: unknown, preset: Record<string, unknown>): Record<string, unknown> {
  const base = isRecord(value) ? value : {};
  const out = { ...base };
  for (const [key, v] of Object.entries(preset)) {
    out[key] = isRecord(v) ? withPreset(base[key], v) : v;
  }
  return out;
}

/** The spec fields the chosen options are about, shown before the rest. */
export function chosenFields(groups: ChoiceGroup[], selection: Selection): string[] {
  return chosen(groups, selection).flatMap((o) => o.fields);
}
