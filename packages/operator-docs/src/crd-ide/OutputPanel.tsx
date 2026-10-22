"use client";

import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { DynamicCodeBlock } from "fumadocs-ui/components/dynamic-codeblock.core";
import { yamlHighlighter, yamlPreviewTheme } from "../crd-form/yaml-theme";
import { exportFiles, exportParts } from "./model/io";
import { zip } from "./model/zip";
import type { Catalog, Issue, Project } from "./model/types";
import { download, headingClass, panelClass } from "./ui";

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** What the project says is wrong, and the manifests it writes. */
export function OutputPanel({
  project,
  catalog,
  issues,
  onSelect,
}: {
  project: Project;
  catalog: Catalog;
  issues: Issue[];
  onSelect: (id: string) => void;
}) {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const { manifests: text, schematic: drawing } = useMemo(() => exportParts(project, catalog), [project, catalog]);
  // The highlighted preview waits for typing to pause; copy and download use `text`.
  const preview = useDeferredValue(text);
  const errors = issues.filter((i) => i.severity === "error").length;
  const warnings = issues.filter((i) => i.severity === "warning").length;

  const show = (state: "copied" | "failed") => {
    setCopyState(state);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopyState("idle"), 1500);
  };
  const copy = () => {
    // Inside the chain: `navigator.clipboard` is undefined outside a secure context.
    Promise.resolve()
      .then(() => navigator.clipboard.writeText(text))
      .then(
        () => show("copied"),
        () => show("failed"),
      );
  };
  const empty = text === "";

  return (
    <section aria-label="Issues and manifests" className={`${panelClass} lg:sticky lg:top-20 lg:self-start`}>
      <h2 className={headingClass}>Issues</h2>
      <p role="status" className="text-[13px] text-fd-foreground">
        {issues.length === 0 ? "No issue." : `${plural(errors, "error")}, ${plural(warnings, "warning")}.`}
      </p>
      {issues.length > 0 ? (
        <ul className="flex max-h-60 flex-col gap-1 overflow-auto text-[13px]">
          {issues.map((issue, i) => {
            const target = issue.resource;
            const mark = issue.severity === "error" ? "Error" : issue.severity === "warning" ? "Warning" : "Note";
            return (
              <li key={i}>
                <span className={issue.severity === "error" ? "text-fd-primary" : "text-fd-muted-foreground"}>{mark}: </span>
                {target ? (
                  <button type="button" className="text-left underline underline-offset-2" onClick={() => onSelect(target)}>
                    {issue.message}
                  </button>
                ) : (
                  issue.message
                )}
              </li>
            );
          })}
        </ul>
      ) : null}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className={headingClass}>Manifests</h2>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="ctl" disabled={empty} onClick={copy}>
            {copyState === "idle" ? "copy" : copyState === "copied" ? "copied" : "copy failed"}
          </button>
          <button type="button" className="ctl" disabled={empty} onClick={() => download("manifests.yaml", text, "text/yaml")}>
            .yaml
          </button>
          <button
            type="button"
            className="ctl"
            disabled={empty}
            aria-label="Download a .zip, one file per resource"
            onClick={() => {
              const bytes = zip(exportFiles(project, catalog));
              download("manifests.zip", new Uint8Array(bytes), "application/zip");
            }}
          >
            .zip
          </button>
        </div>
      </div>
      {drawing ? (
        <p className="flex flex-wrap items-center gap-2 text-[12px] text-fd-muted-foreground">
          Schematic objects are drawn, never applied: they are left out of the manifests.
          <button
            type="button"
            className="ctl"
            aria-label="Download schematic.yaml, the schematic objects to import back"
            onClick={() => download("schematic.yaml", drawing, "text/yaml")}
          >
            schematic.yaml
          </button>
        </p>
      ) : null}
      <span role="status" className="sr-only">
        {copyState === "copied" ? "Manifests copied" : copyState === "failed" ? "Could not copy the manifests" : ""}
      </span>
      <div className="yaml-preview max-h-[60vh] overflow-auto border border-(--rule-soft) text-[13px] [&_pre]:my-0">
        <DynamicCodeBlock lang="yaml" highlighter={yamlHighlighter} code={preview} options={{ theme: yamlPreviewTheme }} />
      </div>
    </section>
  );
}
