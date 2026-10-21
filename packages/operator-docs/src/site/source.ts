import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import { docsContentRoute, docsImageRoute, docsRoute } from "./config";

// The loader options every operator's `src/lib/source.ts` spreads next to its
// own collection (`collections/server` is generated in the operator, so the
// `loader()` call itself stays there):
//
//   export const source = loader({ source: docs.toFumadocsSource(), ...docsLoaderOptions });
//
// See https://fumadocs.dev/docs/headless/source-api for more info.
export const docsLoaderOptions = {
  baseUrl: docsRoute,
  plugins: [lucideIconsPlugin()],
};

/** What the helpers below need from a page, whatever the operator's collection. */
export interface DocsPageLike {
  slugs: string[];
  url: string;
  locale?: string | undefined;
  data: {
    title?: string | undefined;
    description?: string | undefined;
    getText: (type: "processed") => Promise<string>;
  };
}

export function getPageImage(page: Pick<DocsPageLike, "slugs">) {
  const segments = [...page.slugs, "image.png"];

  return {
    segments,
    url: `${docsImageRoute}/${segments.join("/")}`,
  };
}

export function getPageMarkdownUrl(page: Pick<DocsPageLike, "slugs">) {
  const segments = [...page.slugs, "content.md"];

  return {
    segments,
    url: `${docsContentRoute}/${segments.join("/")}`,
  };
}

/** A page as Markdown for `llms.txt`-style consumers (needs `includeProcessedMarkdown`). */
export async function getLLMText(page: DocsPageLike) {
  const processed = await page.data.getText("processed");

  return `# ${page.data.title} (${page.url})

${processed}`;
}
