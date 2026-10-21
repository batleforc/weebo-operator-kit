"use client";

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock.core";
import { ChoicePicker } from "./ChoicePicker";
import { ObjectFields } from "./fields";
import { asRecord, defaultForSchema } from "./schema-utils";
import { buildManifest } from "./manifest";
import { yamlHighlighter, yamlPreviewTheme } from "./yaml-theme";
import {
  applySelection,
  chosenFields,
  defaultSelection,
  hiddenKeys,
  withChoiceRequirements,
  withPreset,
  type ChoiceGroup,
} from "./choices";
import { isCrdSchema, type CrdSchema } from "./types";
import { fetchJson } from "./fetch-json";

const NO_CHOICES: ChoiceGroup[] = [];

const inputClass =
  "w-full border border-(--rule-strong) bg-(--ground-sunk) px-3 py-2 text-[15px] text-fd-foreground placeholder:text-fd-muted-foreground focus:border-(--accent)";

const labelClass = "text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground";

export interface CrdFormProps {
  kind: string;
  /** The kind's guided choices (its entry of the operator's `CrdChoices`). */
  choices?: ChoiceGroup[] | undefined;
}

// The schema is fetched client-side from the static file the same
// `weebo-crd-docs` run already writes to `public/crd-schemas/` — no separate
// data source to keep in sync.
export function CrdForm({ kind, choices }: CrdFormProps) {
  const [schema, setSchema] = useState<CrdSchema | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [namespace, setNamespace] = useState("default");
  const [specValue, setSpecValue] = useState<unknown>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const copyTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const nameId = useId();
  const namespaceId = useId();
  const groups = choices ?? NO_CHOICES;
  const [selection, setSelection] = useState(() => defaultSelection(groups));

  useEffect(() => {
    const controller = new AbortController();
    fetchJson(`/crd-schemas/${kind.toLowerCase()}.schema.json`, isCrdSchema, controller.signal)
      .then((data) => {
        setSchema(data);
        setSpecValue(defaultForSchema(data));
      })
      .catch((err: unknown) => {
        if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err));
      });
    return () => controller.abort();
  }, [kind]);

  useEffect(() => () => clearTimeout(copyTimer.current), []);

  // `spec.name` shows up on many kinds as the remote object's own name — a
  // different concept from metadata.name
  // (the k8s object identifier), but distinct enough in name only, not in
  // UX value: merged into the single "Name" input below rather than shown
  // twice.
  const hasSpecName = schema?.properties?.name?.type === "string" && !schema.properties.name.enum;

  // What the manifest carries: the choices applied (other options' fields
  // dropped, implied values set) and the shared name folded in.
  const effectiveSpecValue = useMemo(() => {
    if (specValue === null) return specValue;
    const spec = applySelection(groups, selection, asRecord(specValue));
    return hasSpecName ? { ...spec, name } : spec;
  }, [specValue, hasSpecName, name, groups, selection]);

  // The chosen options' must-have fields count as required, for the
  // asterisks and for the "still to fill in" list alike.
  const formSchema = useMemo(
    () => (schema ? withChoiceRequirements(groups, selection, schema) : null),
    [schema, groups, selection],
  );

  const built = useMemo(() => {
    if (!formSchema || effectiveSpecValue === null) return null;
    return buildManifest(formSchema, name, namespace, effectiveSpecValue);
  }, [formSchema, effectiveSpecValue, name, namespace]);

  const choose = (groupId: string, optionId: string) => {
    setSelection({ ...selection, [groupId]: optionId });
    const preset = groups.find((g) => g.id === groupId)?.options.find((o) => o.id === optionId)?.preset;
    if (preset) setSpecValue((prev: unknown) => withPreset(prev, preset));
  };

  if (error) {
    return (
      <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">
        Could not load the schema for <code>{kind}</code>: {error}
      </p>
    );
  }

  if (!schema || specValue === null) {
    return (
      <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">
        Loading form...
      </p>
    );
  }

  const showCopyState = (state: "copied" | "failed") => {
    setCopyState(state);
    clearTimeout(copyTimer.current);
    copyTimer.current = setTimeout(() => setCopyState("idle"), 1500);
  };

  const copyYaml = () => {
    if (!built) return;
    const { yaml } = built;
    // Inside the chain: `navigator.clipboard` is undefined outside a secure context.
    Promise.resolve()
      .then(() => navigator.clipboard.writeText(yaml))
      .then(
        () => showCopyState("copied"),
        () => showCopyState("failed"),
      );
  };

  const downloadYaml = () => {
    if (!built) return;
    const blob = new Blob([built.yaml], { type: "text/yaml" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${name || kind.toLowerCase()}.yaml`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="not-prose grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="flex min-w-0 flex-col gap-4">
        {groups.length > 0 ? (
          <ChoicePicker groups={groups} selection={selection} onChange={choose} />
        ) : null}
        <p className="text-[12px] text-fd-muted-foreground">
          <span className="text-fd-primary">*</span> marks a required field.
        </p>
        <div className="border border-(--rule-soft) p-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="flex flex-col gap-1">
              <label htmlFor={nameId} className={labelClass}>
                {hasSpecName ? "name" : "metadata.name"} <span className="text-fd-primary">*</span>
              </label>
              <input
                id={nameId}
                aria-required
                className={inputClass}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-resource"
              />
              {hasSpecName ? (
                <p className="text-[12px] text-fd-muted-foreground">
                  Used as both <code>metadata.name</code> and <code>spec.name</code> — keep it a valid
                  Kubernetes name (lowercase, alphanumeric, hyphens).
                </p>
              ) : null}
            </div>
            {schema.scope === "Namespaced" ? (
              <div className="flex flex-col gap-1">
                <label htmlFor={namespaceId} className={labelClass}>
                  metadata.namespace <span className="text-fd-primary">*</span>
                </label>
                <input
                  id={namespaceId}
                  aria-required
                  className={inputClass}
                  value={namespace}
                  onChange={(e) => setNamespace(e.target.value)}
                  placeholder="default"
                />
              </div>
            ) : null}
          </div>
        </div>
        <ObjectFields
          schema={formSchema ?? schema}
          value={specValue}
          onChange={setSpecValue}
          omitKeys={[...(hasSpecName ? ["name"] : []), ...hiddenKeys(groups, selection)]}
          collapseOptional
          priorityKeys={chosenFields(groups, selection)}
        />
      </div>

      <div className="flex min-w-0 flex-col gap-2 lg:sticky lg:top-20 lg:self-start">
        <div className="flex items-center justify-between">
          <span className={labelClass}>Generated manifest</span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={copyYaml}
              className="ctl"
            >
              {copyState === "idle" ? "copy" : copyState === "copied" ? "copied" : "copy failed"}
            </button>
            <button
              type="button"
              onClick={downloadYaml}
              className="ctl"
            >
              download
            </button>
          </div>
        </div>
        <span role="status" className="sr-only">
          {copyState === "copied" ? "Manifest copied" : copyState === "failed" ? "Could not copy the manifest" : ""}
        </span>
        <span role="status" className="sr-only">
          {built?.missing.length ? `${built.missing.length} required field${built.missing.length === 1 ? "" : "s"} still to fill in` : "Every required field is filled in"}
        </span>
        {built && built.missing.length > 0 ? (
          <div className="border border-(--accent) p-3 text-[13px] text-fd-foreground">
            <p className="mb-1 text-[12px] font-semibold uppercase tracking-[0.1em] text-fd-primary">
              Still to fill in
            </p>
            <ul className="list-inside list-disc">
              {built.missing.map((path) => (
                <li key={path}>
                  <code>{path}</code>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        <div className="yaml-preview max-h-[70vh] overflow-auto border border-(--rule-soft) text-[13px] [&_pre]:my-0">
          <DynamicCodeBlock
            lang="yaml"
            highlighter={yamlHighlighter}
            code={built?.yaml ?? ""}
            options={{ theme: yamlPreviewTheme }}
          />
        </div>
      </div>
    </div>
  );
}
