"use client";

import dynamic from "next/dynamic";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { defaultSelection, type CrdChoices, type Selection } from "../crd-form/choices";
import { loadDraft, saveDraft } from "./draft";
import { DEFAULT_SOURCES, loadCatalog, loadSchemas, type CatalogSource, type SchemaFiles } from "./load";
import { missingSchemas, withSchemas } from "./model/catalog";
import { buildGraph, pointAt } from "./model/graph";
import { importYaml } from "./model/io";
import {
  addExternal,
  addResource,
  emptyProject,
  groupKindOf,
  newDoc,
  removeExternal,
  removeResource,
  setDefaultNamespace,
  updateResource,
} from "./model/project";
import type { Catalog, Issue, Project } from "./model/types";
import { validate } from "./model/validate";
import { OutputPanel } from "./OutputPanel";
import { ResourceEditor } from "./ResourceEditor";
import { ResourceList } from "./ResourceList";
import { inputClass, labelClass, panelClass } from "./ui";

const GraphView = dynamic(() => import("./GraphView"), {
  ssr: false,
  loading: () => <p className="text-[13px] text-fd-muted-foreground">Loading the graph...</p>,
});

/** A ready-made project to start from: multi-document YAML, `<…>` marking what to fill in. */
export interface Recipe {
  id: string;
  label: string;
  description?: string;
  yaml: string;
}

export interface CrdIdeProps {
  /** Where the schemas are; default the site's own `/crd-schemas`. */
  sources?: CatalogSource[] | undefined;
  recipes?: Recipe[] | undefined;
  choices?: CrdChoices | undefined;
  /** The localStorage key of the draft; default one per page. */
  storageKey?: string | undefined;
}

const NO_RECIPES: Recipe[] = [];

/**
 * The CRD IDE: a project of manifests, of one operator or several, linked to
 * each other, edited with the CRD's own form and checked as you type.
 */
export function CrdIde({ sources = DEFAULT_SOURCES, recipes = NO_RECIPES, choices = {}, storageKey }: CrdIdeProps) {
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [files, setFiles] = useState<SchemaFiles>(() => new Map());
  const [error, setError] = useState<string | null>(null);
  const [project, setProject] = useState<Project>(() => emptyProject());
  const [selected, setSelected] = useState<string | undefined>(undefined);
  const [selections, setSelections] = useState<Record<string, Selection>>({});
  const [importIssues, setImportIssues] = useState<Issue[]>([]);
  const [view, setView] = useState<"list" | "graph">("list");
  const editorHeading = useRef<HTMLHeadingElement>(null);
  const sourceKey = sources.map((s) => s.base).join(",");
  const key = storageKey ?? `weebo-crd-ide:${sourceKey}`;

  useEffect(() => {
    const controller = new AbortController();
    loadCatalog(sources, controller.signal)
      .then((loaded) => {
        setCatalog(loaded.catalog);
        setFiles(loaded.files);
        const draft = loadDraft(key);
        if (draft) setProject(draft);
      })
      .catch((err: unknown) => {
        if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err));
      });
    return () => controller.abort();
    // Keyed on the sources' content (`sourceKey`), not their array's identity.
  }, [sourceKey, key]);

  // Saved once typing pauses, and when the page goes away.
  useEffect(() => {
    if (!catalog) return;
    const save = () => saveDraft(key, project);
    const timer = setTimeout(save, 500);
    window.addEventListener("pagehide", save);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("pagehide", save);
    };
  }, [catalog, key, project]);

  // A kind's schema loads with its first resource: the catalog keeps only
  // the index and the links of the kinds nobody uses.
  const missing = useMemo(
    () => (catalog ? missingSchemas(catalog, project.resources.flatMap((r) => groupKindOf(r.doc) ?? [])) : []),
    [catalog, project],
  );
  const missingKey = missing.join(",");
  useEffect(() => {
    if (!missing.length) return;
    const controller = new AbortController();
    loadSchemas(files, missing, controller.signal)
      .then((schemas) => setCatalog((current) => (current ? withSchemas(current, schemas) : current)))
      .catch((err: unknown) => {
        if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err));
      });
    return () => controller.abort();
    // Keyed on the kinds' names (`missingKey`), not the array's identity.
  }, [missingKey, files]);

  const graph = useMemo(() => (catalog ? buildGraph(project, catalog) : null), [project, catalog]);
  const issues = useMemo(
    () => (catalog && graph ? [...importIssues, ...validate(project, catalog, graph)] : []),
    [project, catalog, graph, importIssues],
  );

  if (error) {
    return <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">Could not load the CRDs: {error}</p>;
  }
  if (!catalog || !graph) {
    return <p className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">Loading the CRDs...</p>;
  }

  const resource = project.resources.find((r) => r.id === selected);
  const gk = resource && groupKindOf(resource.doc);
  const kindChoices = (gk && choices[gk.kind]) ?? [];
  // From an issue: the editor may be off-screen, focus goes to it.
  const openInEditor = (id: string) => {
    setSelected(id);
    setTimeout(() => editorHeading.current?.focus(), 0);
  };
  const runImport = (text: string) => {
    const result = importYaml(project, catalog, text);
    setProject(result.project);
    setImportIssues(result.issues);
    const first = result.project.resources[project.resources.length];
    if (first) setSelected(first.id);
  };

  return (
    // A container: the columns follow the IDE's own width (the docs column), not the window's.
    <div className="not-prose @container flex flex-col gap-4">
      <Toolbar
        catalog={catalog}
        project={project}
        recipes={recipes}
        view={view}
        onView={setView}
        onAdd={(kindKey, name) => {
          const entry = catalog.get(kindKey);
          if (!entry) return;
          const added = addResource(project, newDoc(catalog, { group: entry.group, kind: entry.kind }, name));
          setProject(added.project);
          setSelected(added.id);
        }}
        onImport={runImport}
        onNamespace={(ns) => setProject(setDefaultNamespace(project, ns))}
        onClear={() => {
          setProject(emptyProject(project.defaultNamespace));
          setImportIssues([]);
          setSelected(undefined);
        }}
      />
      {view === "graph" ? <GraphView graph={graph} selected={selected} onSelect={setSelected} /> : null}
      <div className="grid gap-4 @3xl:grid-cols-[18rem_minmax(0,1fr)] @7xl:grid-cols-[18rem_minmax(0,1fr)_minmax(0,1fr)]">
        {/* Shown in both views: the list is where every action is. */}
        <ResourceList
            project={project}
            catalog={catalog}
            graph={graph}
            issues={issues}
            selected={selected}
            onSelect={setSelected}
            onRemove={(id) => {
              setProject(removeResource(project, id));
              if (selected === id) setSelected(undefined);
            }}
            onPoint={(slot, target) => {
              const doc = pointAt(project, catalog, slot, target);
              if (doc) setProject(updateResource(project, slot.resource, doc));
            }}
            onAddExternal={(x) => setProject(addExternal(project, x).project)}
            onRemoveExternal={(id) => setProject(removeExternal(project, id))}
          />
        {resource ? (
          <ResourceEditor
            key={resource.id}
            project={project}
            catalog={catalog}
            resource={resource}
            choices={kindChoices}
            selection={selections[resource.id] ?? defaultSelection(kindChoices)}
            onSelection={(s) => setSelections({ ...selections, [resource.id]: s })}
            onChange={(doc) => setProject(updateResource(project, resource.id, doc))}
            headingRef={editorHeading}
          />
        ) : (
          <p className={`${panelClass} text-[13px] text-fd-muted-foreground`}>Select a resource to edit it.</p>
        )}
        <div className="@3xl:col-span-2 @7xl:col-span-1">
          <OutputPanel project={project} catalog={catalog} issues={issues} onSelect={openInEditor} />
        </div>
      </div>
    </div>
  );
}

function Toolbar({
  catalog,
  project,
  recipes,
  view,
  onView,
  onAdd,
  onImport,
  onNamespace,
  onClear,
}: {
  catalog: Catalog;
  project: Project;
  recipes: Recipe[];
  view: "list" | "graph";
  onView: (view: "list" | "graph") => void;
  onAdd: (kindKey: string, name: string) => void;
  onImport: (text: string) => void;
  onNamespace: (namespace: string) => void;
  onClear: () => void;
}) {
  const kinds = [...catalog.entries()].sort(([, a], [, b]) => a.kind.localeCompare(b.kind));
  const [kind, setKind] = useState(kinds[0]?.[0] ?? "");
  const [name, setName] = useState("");
  const [recipe, setRecipe] = useState(recipes[0]?.id ?? "");
  const [pasted, setPasted] = useState("");
  const kindSelect = useRef<HTMLSelectElement>(null);
  const ids = { kind: useId(), name: useId(), ns: useId(), recipe: useId(), file: useId(), paste: useId() };
  const groups = new Set(kinds.map(([, e]) => e.group));
  const chosenRecipe = recipes.find((r) => r.id === recipe);

  return (
    <div className={panelClass}>
      <div className="flex flex-wrap items-end gap-3">
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            onAdd(kind, name || "<name>");
            setName("");
          }}
        >
          <div className="flex flex-col gap-1">
            <label htmlFor={ids.kind} className={labelClass}>
              Kind
            </label>
            <select id={ids.kind} ref={kindSelect} className={inputClass} value={kind} onChange={(e) => setKind(e.target.value)}>
              {kinds.map(([k, e]) => (
                <option key={k} value={k}>
                  {e.kind}
                  {groups.size > 1 ? ` (${e.group})` : ""}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor={ids.name} className={labelClass}>
              Name
            </label>
            <input id={ids.name} className={inputClass} value={name} placeholder="my-resource" onChange={(e) => setName(e.target.value)} />
          </div>
          <button type="submit" className="action" disabled={!kind}>
            Add
          </button>
        </form>
        <div className="flex flex-col gap-1">
          <label htmlFor={ids.ns} className={labelClass}>
            Default namespace
          </label>
          <input id={ids.ns} className={inputClass} value={project.defaultNamespace} onChange={(e) => onNamespace(e.target.value)} />
        </div>
        <div className="flex gap-2" role="group" aria-label="View">
          <button type="button" className="ctl" aria-pressed={view === "list"} onClick={() => onView("list")}>
            list
          </button>
          <button type="button" className="ctl" aria-pressed={view === "graph"} onClick={() => onView("graph")}>
            graph
          </button>
        </div>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        {recipes.length > 0 ? (
          <div className="flex flex-wrap items-end gap-2">
            <div className="flex flex-col gap-1">
              <label htmlFor={ids.recipe} className={labelClass}>
                Recipe
              </label>
              <select id={ids.recipe} className={inputClass} value={recipe} aria-describedby={`${ids.recipe}-desc`} onChange={(e) => setRecipe(e.target.value)}>
                {recipes.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.label}
                  </option>
                ))}
              </select>
            </div>
            <button type="button" className="ctl" onClick={() => chosenRecipe && onImport(chosenRecipe.yaml)}>
              add recipe
            </button>
            <p id={`${ids.recipe}-desc`} className="w-full text-[12px] text-fd-muted-foreground">
              {chosenRecipe?.description}
            </p>
          </div>
        ) : null}
        <div className="flex flex-col gap-1">
          <label htmlFor={ids.file} className={labelClass}>
            Import YAML files
          </label>
          <input
            id={ids.file}
            type="file"
            multiple
            accept=".yaml,.yml,text/yaml"
            className="text-[13px]"
            aria-describedby={`${ids.file}-hint`}
            onChange={(e) => {
              const files = [...(e.target.files ?? [])];
              // One import for them all: references across files resolve.
              if (files.length) Promise.all(files.map((f) => f.text())).then((texts) => onImport(texts.join("\n---\n")), () => onImport(""));
              e.target.value = "";
            }}
          />
          <p id={`${ids.file}-hint`} className="max-w-md text-[12px] text-fd-muted-foreground">
            Manifests, or a cluster&apos;s objects: <code>kubectl get &lt;kinds&gt; -A -o yaml</code>. What the server sets is dropped; a Secret keeps its keys, never its values. An object already in the project is replaced.
          </p>
        </div>
        <button type="button" className="ctl" disabled={project.resources.length === 0 && project.externals.length === 0} onClick={() => {
          if (!window.confirm("Remove every resource and external object of the project?")) return;
          onClear();
          // The button is disabled once the project is empty: focus moves on.
          kindSelect.current?.focus();
        }}>
          clear project
        </button>
      </div>
      <details>
        <summary className="cursor-pointer text-[13px] text-fd-foreground">Paste YAML</summary>
        <div className="mt-2 flex flex-col gap-2">
          <label htmlFor={ids.paste} className={labelClass}>
            Manifests (multi-document YAML)
          </label>
          <textarea id={ids.paste} className={`${inputClass} min-h-40 font-mono text-[13px]`} value={pasted} onChange={(e) => setPasted(e.target.value)} />
          <button
            type="button"
            className="ctl self-start"
            disabled={!pasted.trim()}
            onClick={() => {
              onImport(pasted);
              setPasted("");
            }}
          >
            import
          </button>
        </div>
      </details>
    </div>
  );
}

