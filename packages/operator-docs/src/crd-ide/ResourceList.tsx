"use client";

import { useId, useRef, useState } from "react";
import { candidates, emptySlots, type Graph, type GraphNode, type Slot } from "./model/graph";
import { formatPath } from "./model/paths";
import { isSchematic } from "./model/catalog";
import { groupKindOf, nameOf } from "./model/project";
import type { Catalog, GroupKind, Issue, Project } from "./model/types";
import { headingClass, inputClass, labelClass, panelClass } from "./ui";

const nodeLabel = (n: GraphNode) => `${n.kind.kind} ${n.namespace ? `${n.namespace}/` : ""}${n.name}`;

export interface ResourceListProps {
  project: Project;
  catalog: Catalog;
  graph: Graph;
  issues: Issue[];
  selected: string | undefined;
  onSelect: (id: string) => void;
  onRemove: (id: string) => void;
  onPoint: (slot: Slot, target: GraphNode | undefined) => void;
  onAddExternal: (external: { kind: GroupKind; name: string; namespace?: string; keys: string[] }) => void;
  onRemoveExternal: (id: string) => void;
}

/**
 * The project as a list, the IDE's main view: every resource, its reference
 * fields each with the object it points at (a select, so a link is made
 * with the keyboard alone), then the external objects.
 */
export function ResourceList(props: ResourceListProps) {
  const { project, catalog, graph, issues, selected, onSelect, onRemove, onPoint } = props;
  const headingId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const rows = useRef(new Map<string, HTMLButtonElement>());
  // The removed row's button goes with it: focus moves to the next row, or
  // to the heading when none is left.
  const remove = (id: string) => {
    const index = project.resources.findIndex((r) => r.id === id);
    const next = project.resources[index + 1] ?? project.resources[index - 1];
    onRemove(id);
    setTimeout(() => ((next && rows.current.get(next.id)) ?? heading.current)?.focus(), 0);
  };
  return (
    <section aria-labelledby={headingId} className={panelClass}>
      <h2 id={headingId} ref={heading} tabIndex={-1} className={headingClass}>
        Resources
      </h2>
      {project.resources.length === 0 ? (
        <p className="text-[13px] text-fd-muted-foreground">Empty project: add a resource, a recipe or import YAML.</p>
      ) : (
        <ul className="flex flex-col gap-3">
          {project.resources.map((resource) => {
            const errors = issues.filter((i) => i.resource === resource.id && i.severity === "error").length;
            const label = `${String(resource.doc.kind)} ${nameOf(resource.doc) || "(unnamed)"}`;
            const slots = [
              ...graph.slots.filter((s) => s.resource === resource.id),
              ...emptySlots(graph, project, catalog, resource.id),
            ];
            return (
              <li key={resource.id} className="flex flex-col gap-2 border-l-2 border-(--rule-soft) pl-2 has-[[aria-current=true]]:border-(--accent)">
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    ref={(el) => {
                      if (el) rows.current.set(resource.id, el);
                      else rows.current.delete(resource.id);
                    }}
                    className="min-w-0 flex-1 text-left text-[14px] break-words text-fd-foreground underline-offset-2 hover:underline"
                    aria-current={resource.id === selected}
                    onClick={() => onSelect(resource.id)}
                  >
                    {label}
                    {resource.mode !== "editable" ? <span className="text-fd-muted-foreground"> (read-only)</span> : null}
                    {isSchematic(catalog, groupKindOf(resource.doc) ?? { group: "", kind: "" }) ? (
                      <span className="text-fd-muted-foreground"> (schematic)</span>
                    ) : null}
                  </button>
                  {errors > 0 ? (
                    <span className="text-[12px] text-fd-primary">
                      {errors} error{errors === 1 ? "" : "s"}
                    </span>
                  ) : null}
                  <button type="button" className="ctl" aria-label={`Remove ${label}`} onClick={() => remove(resource.id)}>
                    remove
                  </button>
                </div>
                {slots.map((slot) => (
                  <SlotPicker
                    key={formatPath(slot.path)}
                    owner={label}
                    slot={slot}
                    graph={graph}
                    disabled={resource.mode !== "editable"}
                    onPoint={onPoint}
                  />
                ))}
              </li>
            );
          })}
        </ul>
      )}
      <Externals {...props} />
    </section>
  );
}

function SlotPicker({
  owner,
  slot,
  graph,
  disabled,
  onPoint,
}: {
  /** The resource the field belongs to, for the select's name. */
  owner: string;
  slot: Slot;
  graph: Graph;
  disabled: boolean;
  onPoint: ResourceListProps["onPoint"];
}) {
  const id = useId();
  const options = candidates(graph, slot);
  // A name that matches nothing in the project stays visible as such.
  const dangling = slot.ref && !slot.target ? slot.ref.name : undefined;
  return (
    <div className="flex flex-col gap-1 pl-2">
      <label htmlFor={id} className="text-[12px] text-fd-muted-foreground">
        <span className="sr-only">{owner}: </span>
        <code>{formatPath(slot.path)}</code> → {slot.link.to.map((k) => k.kind).join(" or ")}
      </label>
      <select
        id={id}
        className={inputClass}
        disabled={disabled}
        value={slot.target?.id ?? (dangling ? "dangling" : "")}
        onChange={(e) => {
          if (e.target.value !== "dangling") onPoint(slot, options.find((n) => n.id === e.target.value));
        }}
      >
        <option value="">— none —</option>
        {dangling ? <option value="dangling">{dangling} (not in the project)</option> : null}
        {options.map((n) => (
          <option key={n.id} value={n.id}>
            {nodeLabel(n)}
            {n.origin === "produced" ? " (made)" : n.origin === "external" ? " (external)" : ""}
            {n.schematic ? " (schematic)" : ""}
          </option>
        ))}
      </select>
    </div>
  );
}

function Externals({ project, onAddExternal, onRemoveExternal }: ResourceListProps) {
  const [kind, setKind] = useState("Secret");
  const [group, setGroup] = useState("");
  const [name, setName] = useState("");
  const [namespace, setNamespace] = useState("");
  const [keys, setKeys] = useState("");
  const ids = { kind: useId(), group: useId(), name: useId(), namespace: useId(), keys: useId() };
  const heading = useRef<HTMLHeadingElement>(null);
  const add = () => {
    if (!name || !kind) return;
    onAddExternal({
      kind: { group, kind },
      name,
      ...(namespace ? { namespace } : {}),
      keys: keys.split(",").map((k) => k.trim()).filter(Boolean),
    });
    setName("");
    setKeys("");
  };
  const field = (id: string, label: string, value: string, set: (v: string) => void, placeholder: string) => (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-[12px] text-fd-muted-foreground">
        {label}
      </label>
      <input id={id} className={inputClass} value={value} placeholder={placeholder} onChange={(e) => set(e.target.value)} />
    </div>
  );
  return (
    <section aria-labelledby={`${ids.kind}-title`} className="flex flex-col gap-2 border-t border-(--rule-soft) pt-3">
      <h2 id={`${ids.kind}-title`} ref={heading} tabIndex={-1} className={headingClass}>
        External objects
      </h2>
      <p className="text-[12px] text-fd-muted-foreground">Objects that exist outside the project, for references to point at. Never exported.</p>
      {project.externals.length > 0 ? (
        <ul className="flex flex-col gap-1">
          {project.externals.map((x) => {
            const label = `${x.kind.kind} ${x.namespace ? `${x.namespace}/` : ""}${x.name}`;
            return (
              <li key={x.id} className="flex items-center gap-2 text-[14px]">
                <span className="min-w-0 flex-1 break-words">
                  {label}
                  {x.keys.length ? <span className="text-fd-muted-foreground"> ({x.keys.join(", ")})</span> : null}
                </span>
                <button type="button" className="ctl" aria-label={`Remove ${label}`} onClick={() => {
                    onRemoveExternal(x.id);
                    // Its button goes with it: focus stays in the section.
                    setTimeout(() => heading.current?.focus(), 0);
                  }}
                >
                  remove
                </button>
              </li>
            );
          })}
        </ul>
      ) : null}
      <details>
        <summary className="cursor-pointer text-[13px] text-fd-foreground">Declare an external object</summary>
        <form
          className="mt-2 grid gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          {field(ids.kind, "Kind", kind, setKind, "Secret")}
          {field(ids.group, "API group (empty: core)", group, setGroup, "")}
          {field(ids.name, "Name", name, setName, "my-secret")}
          {field(ids.namespace, "Namespace (empty: the project's)", namespace, setNamespace, "")}
          {field(ids.keys, "Keys, comma-separated", keys, setKeys, "username, password")}
          <button type="submit" className="ctl justify-self-start">
            declare
          </button>
        </form>
      </details>
    </section>
  );
}
