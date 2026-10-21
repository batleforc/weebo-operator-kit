// Link paths (`spec.keys[].name`, `spec.byName.*`) against concrete values.

import { isRecord } from "../../crd-form/types";
import type { Segment } from "./types";

/** A link path's steps: a property, `[]` (every item) or `*` (every map value). */
type Step = { prop: string } | { each: "item" | "value" };

export function parseLinkPath(path: string): Step[] {
  const steps: Step[] = [];
  for (const part of path.split(".")) {
    const match = /^([^[\]]+)((?:\[\])*)$/.exec(part);
    const name = match?.[1];
    if (name === undefined) throw new Error(`invalid link path ${JSON.stringify(path)}`);
    steps.push(name === "*" ? { each: "value" } : { prop: name });
    for (let i = 0; i < (match?.[2]?.length ?? 0) / 2; i++) steps.push({ each: "item" });
  }
  return steps;
}

/** Every value a link path reaches in `doc`, with its concrete path. Unset values are skipped. */
export function expand(doc: unknown, path: string): { path: Segment[]; value: unknown }[] {
  let found: { path: Segment[]; value: unknown }[] = [{ path: [], value: doc }];
  for (const step of parseLinkPath(path)) {
    found = found.flatMap(({ path: at, value }) => {
      if ("prop" in step) {
        return isRecord(value) && value[step.prop] !== undefined
          ? [{ path: [...at, step.prop], value: value[step.prop] }]
          : [];
      }
      if (step.each === "item") {
        return Array.isArray(value) ? value.map((item: unknown, i) => ({ path: [...at, i], value: item })) : [];
      }
      return isRecord(value) ? Object.entries(value).map(([k, v]) => ({ path: [...at, k], value: v })) : [];
    });
  }
  return found;
}

export function getAt(doc: unknown, path: Segment[]): unknown {
  let value = doc;
  for (const step of path) {
    if (typeof step === "number") value = Array.isArray(value) ? value[step] : undefined;
    else value = isRecord(value) ? value[step] : undefined;
  }
  return value;
}

/** `doc` with `value` at `path`, missing objects created; `doc` itself is left untouched. */
export function setAt(doc: unknown, path: Segment[], value: unknown): unknown {
  const [step, ...rest] = path;
  if (step === undefined) return value;
  if (typeof step === "number") {
    const list = Array.isArray(doc) ? [...doc] : [];
    list[step] = setAt(list[step], rest, value);
    return list;
  }
  const record = isRecord(doc) ? { ...doc } : {};
  record[step] = setAt(record[step], rest, value);
  return record;
}

/** `spec.keys[0].name`, for messages and issue paths. */
export function formatPath(path: Segment[]): string {
  return path.reduce<string>(
    (out, step) => (typeof step === "number" ? `${out}[${step}]` : out ? `${out}.${step}` : step),
    "",
  );
}
