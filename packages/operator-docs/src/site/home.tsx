import type { ReactNode } from "react";
import Link from "next/link";
import { repoUrl, type SiteConfig } from "./config";

// The proof sheet: one form at poster scale captioned by its facts, then
// ruled regions — no card, no glow, no gradient (DESIGN.md, Overview).
// Every operator's landing page is this one with its own words.

export interface HomeAction {
  href: string;
  label: string;
  /** The filled button; the others are outlined. */
  primary?: boolean;
}

export interface HomeFeature {
  title: string;
  body: ReactNode;
  href: string;
  link: string;
}

export interface OperatorHomeProps {
  site: SiteConfig;
  /** The small line above the name ("Kubernetes operator · <product>"). */
  label: string;
  caption: ReactNode;
  /** Buttons under the caption; "View on GitHub" is always appended. */
  actions: HomeAction[];
  /** Shell lines of "1 · Install the chart". */
  install: string;
  /** "2 · <title>": a short manifest, as YAML. */
  example: { title: string; yaml: string };
  features: HomeFeature[];
}

// `key: value` lines drawn with the sheet's key/value colours; anything
// else (list items, block scalars) is printed as is.
function YamlSheet({ yaml }: { yaml: string }) {
  const lines = yaml.replace(/\n+$/, "").split("\n");
  return (
    <>
      {lines.map((line, i) => {
        const m = /^(\s*)([^\s:#][^:]*:)(?:\s+(.*))?$/.exec(line);
        return (
          <span key={i}>
            {i > 0 ? "\n" : null}
            {m ? (
              <>
                {m[1]}
                <span className="k">{m[2]}</span>
                {m[3] ? (
                  <>
                    {" "}
                    <span className="v">{m[3]}</span>
                  </>
                ) : null}
              </>
            ) : (
              line
            )}
          </span>
        );
      })}
    </>
  );
}

export function OperatorHome({ site, label, caption, actions, install, example, features }: OperatorHomeProps) {
  return (
    <main className="flex flex-1 flex-col px-4 sm:px-6">
      <section className="mx-auto flex w-full max-w-[1100px] flex-col gap-6 pt-10 pb-10 sm:pt-16">
        <span className="sheet-label">{label}</span>
        <h1 className="sheet-display">{site.name}</h1>
        <p className="sheet-caption">{caption}</p>
        <div className="flex flex-wrap items-center gap-3">
          {actions.map((action) => (
            <Link key={action.href} href={action.href} className={action.primary ? "action" : "ctl"}>
              {action.label}
            </Link>
          ))}
          <a href={repoUrl(site)} className="ctl" rel="noreferrer">
            View on GitHub
          </a>
        </div>
      </section>

      <section className="sheet-region mx-auto w-full max-w-[1100px]">
        <div className="grid gap-6 md:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-3">
            <span className="sheet-label">1 · Install the chart</span>
            <pre className="sheet-pre">
              <code>{install.replace(/\n+$/, "")}</code>
            </pre>
          </div>
          <div className="flex min-w-0 flex-col gap-3">
            <span className="sheet-label">2 · {example.title}</span>
            <pre className="sheet-pre">
              <code>
                <YamlSheet yaml={example.yaml} />
              </code>
            </pre>
          </div>
        </div>
      </section>

      <section className="sheet-region mx-auto grid w-full max-w-[1100px] md:grid-cols-3">
        {features.map((f) => (
          <div key={f.title} className="sheet-feature flex flex-col gap-3 py-6 md:px-6 md:py-0 md:first:ps-0">
            <h2>{f.title}</h2>
            <p className="text-[13px] leading-[1.6] text-fd-muted-foreground">{f.body}</p>
            <Link href={f.href}>{f.link}</Link>
          </div>
        ))}
      </section>
    </main>
  );
}
