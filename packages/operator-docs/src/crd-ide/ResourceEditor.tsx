"use client";

import { useId, type Ref } from "react";
import * as yaml from "js-yaml";
import { ChoicePicker } from "../crd-form/ChoicePicker";
import { applySelection, chosenFields, hiddenKeys, withChoiceRequirements, withPreset, type ChoiceGroup, type Selection } from "../crd-form/choices";
import { ObjectFields } from "../crd-form/fields";
import { asRecord } from "../crd-form/schema-utils";
import { isRecord } from "../crd-form/types";
import { gkKey } from "./model/catalog";
import { groupKindOf, nameOf } from "./model/project";
import type { Catalog, Doc, Project, Resource } from "./model/types";
import { headingClass, inputClass, labelClass, panelClass } from "./ui";

export interface ResourceEditorProps {
  project: Project;
  catalog: Catalog;
  resource: Resource;
  choices: ChoiceGroup[];
  selection: Selection;
  onSelection: (selection: Selection) => void;
  onChange: (doc: Doc) => void;
  /** The title, focusable, for the IDE to move focus to the editor. */
  headingRef?: Ref<HTMLHeadingElement> | undefined;
}

/** The selected resource: its name and namespace, then `crd-form`'s fields for its spec. */
export function ResourceEditor({ project, catalog, resource, choices, selection, onSelection, onChange, headingRef }: ResourceEditorProps) {
  const nameId = useId();
  const namespaceId = useId();
  const titleId = useId();
  const gk = groupKindOf(resource.doc);
  const entry = gk && catalog.get(gkKey(gk));
  const title = `${String(resource.doc.kind)} ${nameOf(resource.doc) || "(unnamed)"}`;

  if (resource.mode !== "editable" || !entry) {
    return (
      <section aria-labelledby={titleId} className={panelClass}>
        <h2 id={titleId} ref={headingRef} tabIndex={-1} className={headingClass}>
          {title}
        </h2>
        <p className="text-[13px] text-fd-muted-foreground">
          {resource.mode === "unknown"
            ? "A kind outside the catalog: kept as imported and exported unchanged."
            : `Another version than ${entry?.apiVersion ?? "the catalog's"}: kept as imported, read-only.`}
        </p>
        <pre className="overflow-auto border border-(--rule-soft) p-3 text-[13px]">{yaml.dump(resource.doc, { lineWidth: 100 })}</pre>
      </section>
    );
  }

  const { schema } = entry;
  if (!schema) {
    return (
      <section aria-labelledby={titleId} className={panelClass}>
        <h2 id={titleId} ref={headingRef} tabIndex={-1} className={headingClass}>
          {title}
        </h2>
        <p role="status" className="text-[13px] text-fd-muted-foreground">
          Loading the schema of {entry.kind}...
        </p>
      </section>
    );
  }
  const metadata = isRecord(resource.doc.metadata) ? resource.doc.metadata : {};
  const setMetadata = (key: "name" | "namespace", value: string) => {
    const next: Record<string, unknown> = { ...metadata, [key]: value };
    if (key === "namespace" && !value) delete next.namespace;
    onChange({ ...resource.doc, metadata: next });
  };
  const setSpec = (spec: Record<string, unknown>) => onChange({ ...resource.doc, spec: applySelection(choices, selection, spec) });
  const choose = (groupId: string, optionId: string) => {
    onSelection({ ...selection, [groupId]: optionId });
    const preset = choices.find((g) => g.id === groupId)?.options.find((o) => o.id === optionId)?.preset;
    if (preset) setSpec(asRecord(withPreset(resource.doc.spec, preset)));
  };
  const namespace = typeof metadata.namespace === "string" ? metadata.namespace : "";

  return (
    <section aria-labelledby={titleId} className={panelClass}>
      <h2 id={titleId} ref={headingRef} tabIndex={-1} className={headingClass}>
        {title}
      </h2>
      {entry.schematic ? (
        <p className="text-[12px] text-fd-muted-foreground">
          A schematic kind: drawn and linked here, never applied. Exported in <code>schematic.yaml</code>, not with the manifests.
        </p>
      ) : null}
      {choices.length > 0 ? <ChoicePicker groups={choices} selection={selection} onChange={choose} /> : null}
      <p className="text-[12px] text-fd-muted-foreground">
        <span className="text-fd-primary">*</span> marks a required field.
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="flex flex-col gap-1">
          <label htmlFor={nameId} className={labelClass}>
            metadata.name <span className="text-fd-primary">*</span>
          </label>
          <input id={nameId} aria-required className={inputClass} value={nameOf(resource.doc)} onChange={(e) => setMetadata("name", e.target.value)} />
        </div>
        {entry.scope === "Namespaced" ? (
          <div className="flex flex-col gap-1">
            <label htmlFor={namespaceId} className={labelClass}>
              metadata.namespace
            </label>
            <input
              id={namespaceId}
              className={inputClass}
              value={namespace}
              placeholder={project.defaultNamespace}
              aria-describedby={`${namespaceId}-hint`}
              onChange={(e) => setMetadata("namespace", e.target.value)}
            />
            <p id={`${namespaceId}-hint`} className="text-[12px] text-fd-muted-foreground">
              Empty: the project's default namespace.
            </p>
          </div>
        ) : null}
      </div>
      <ObjectFields
        schema={withChoiceRequirements(choices, selection, schema)}
        value={resource.doc.spec ?? {}}
        onChange={setSpec}
        omitKeys={hiddenKeys(choices, selection)}
        collapseOptional
        priorityKeys={chosenFields(choices, selection)}
      />
    </section>
  );
}
