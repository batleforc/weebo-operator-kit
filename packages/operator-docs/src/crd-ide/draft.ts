// The project in progress, kept in this browser between visits. A
// convenience only: storage may be missing or full, and the page works
// without it.

import { isRecord } from "../crd-form/types";
import type { Project } from "./model/types";

const isProject = (v: unknown): v is Project =>
  isRecord(v) &&
  typeof v.defaultNamespace === "string" &&
  Array.isArray(v.resources) &&
  v.resources.every(
    (r) => isRecord(r) && typeof r.id === "string" && isRecord(r.doc) && (r.mode === "editable" || r.mode === "readOnly" || r.mode === "unknown"),
  ) &&
  Array.isArray(v.externals) &&
  v.externals.every(
    (x) =>
      isRecord(x) &&
      typeof x.id === "string" &&
      typeof x.name === "string" &&
      isRecord(x.kind) &&
      typeof x.kind.group === "string" &&
      typeof x.kind.kind === "string" &&
      Array.isArray(x.keys),
  );

export function loadDraft(key: string): Project | undefined {
  try {
    const raw = window.localStorage.getItem(key);
    const data: unknown = raw ? JSON.parse(raw) : undefined;
    return isRecord(data) && data.v === 1 && isProject(data.project) ? data.project : undefined;
  } catch {
    return undefined;
  }
}

export function saveDraft(key: string, project: Project): void {
  try {
    window.localStorage.setItem(key, JSON.stringify({ v: 1, project }));
  } catch {
    // No storage (private window, quota): the draft is not kept, nothing else changes.
  }
}
