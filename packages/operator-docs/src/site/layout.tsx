import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { repoUrl, type SiteConfig } from "./config";

// The wordmark's mark: a 3×3 grid of 4px squares on a 16px viewBox with 2px
// gutters — the dot grid is the icon system (DESIGN.md, Shapes).
function DotMark() {
  const cells = [1, 7, 13].flatMap((y) => [1, 7, 13].map((x) => [x, y] as const));
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" fill="currentColor">
      {cells.map(([x, y]) => (
        <rect key={`${x}-${y}`} x={x - 1} y={y - 1} width="4" height="4" />
      ))}
    </svg>
  );
}

// The display face is wide: past this many characters the name at 16px
// overflows the sidebar header and pushes its collapse button over the page.
const LONG_NAME = 15;

/** Options shared by the home and docs layouts: the wordmark, the repo link, the theme switch. */
export function baseOptions(site: SiteConfig): BaseLayoutProps {
  const size = site.name.length > LONG_NAME ? "text-[12px]" : "text-[16px]";
  return {
    nav: {
      title: (
        <span
          className={`inline-flex min-w-0 items-center gap-2 whitespace-nowrap ${size} font-bold uppercase tracking-[0.04em]`}
          style={{ fontFamily: "var(--face-display)", color: "var(--ink)" }}
          title={site.name}
        >
          <DotMark />
          <span className="truncate">{site.name}</span>
        </span>
      ),
    },
    githubUrl: repoUrl(site),
    themeSwitch: { mode: "light-dark-system" },
  };
}
