// What varies between two operators' docs sites: a name, a one-line pitch and
// the repository the pages are edited in. Everything else (routes, theme,
// layout) is the same for every operator.

export interface SiteConfig {
  /** The operator's name, shown in the nav, the titles and the OG images. */
  name: string;
  /** One sentence: the site's default `<meta name="description">`. */
  description: string;
  git: {
    user: string;
    repo: string;
    branch: string;
    /** Where `content/docs` lives in the repository ("Edit on GitHub" links). */
    contentDir: string;
  };
}

export function defineSite(config: SiteConfig): SiteConfig {
  return config;
}

export function repoUrl(site: SiteConfig): string {
  return `https://github.com/${site.git.user}/${site.git.repo}`;
}

// The routes every operator's site serves: the expected tree under
// `content/docs` lands at `docsRoute`, each page's OG image and Markdown
// rendition next to it.
export const docsRoute = "/docs";
export const docsImageRoute = "/og/docs";
export const docsContentRoute = "/llms.mdx/docs";
