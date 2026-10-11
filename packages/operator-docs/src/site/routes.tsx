// Route handlers of every operator's site, as factories over the operator's
// `source`. Next reads route segment config (`revalidate`, `dynamic`)
// statically from the route file itself, so those stay in the operator's
// files; only the handlers come from here. Everything is pre-rendered:
// `output: 'export'` has no server (GitHub Pages).
import { notFound } from "next/navigation";
import { ImageResponse } from "next/og";
import { llms, type LoaderConfig, type LoaderOutput } from "fumadocs-core/source";
import { createFromSource } from "fumadocs-core/search/server";
import { generate as DefaultImage } from "fumadocs-ui/og";
import type { SiteConfig } from "./config";
import { getLLMText, getPageImage, getPageMarkdownUrl, type DocsPageLike } from "./source";

/** What the handlers need from the operator's `source`. */
interface DocsSourceLike<P extends DocsPageLike> {
  getPages: () => P[];
  getPage: (slugs: string[] | undefined) => P | undefined;
}

/**
 * `app/api/search/route.ts`: the search index, exported once at build time
 * and queried in the browser (`DocsRootProvider` sets `type: 'static'`).
 */
export function searchRoute<C extends LoaderConfig>(source: LoaderOutput<C>) {
  // https://docs.orama.com/docs/orama-js/supported-languages
  return createFromSource(source, { language: "english" }).staticGET;
}

/** `app/llms.txt/route.ts`: the index of every page. */
export function llmsIndexRoute<C extends LoaderConfig>(source: LoaderOutput<C>) {
  return async function GET() {
    return new Response(await llms(source).index());
  };
}

/** `app/llms-full.txt/route.ts`: every page as Markdown, in one file. */
export function llmsFullRoute<P extends DocsPageLike>(source: DocsSourceLike<P>) {
  return async function GET() {
    const scanned = await Promise.all(source.getPages().map(getLLMText));

    return new Response(scanned.join("\n\n"));
  };
}

/** `app/llms.mdx/docs/[[...slug]]/route.ts`: one page as Markdown. */
export function llmsPageRoute<P extends DocsPageLike>(source: DocsSourceLike<P>) {
  async function GET(_req: Request, { params }: { params: Promise<{ slug?: string[] }> }) {
    const { slug } = await params;
    const page = source.getPage(slug?.slice(0, -1));
    if (!page) notFound();

    return new Response(await getLLMText(page), {
      headers: {
        "Content-Type": "text/markdown",
      },
    });
  }

  function generateStaticParams() {
    return source.getPages().map((page) => ({
      slug: getPageMarkdownUrl(page).segments,
    }));
  }

  return { GET, generateStaticParams };
}

/** `app/og/docs/[...slug]/route.tsx`: each page's Open Graph image. */
export function ogRoute<P extends DocsPageLike>(source: DocsSourceLike<P>, site: SiteConfig) {
  async function GET(_req: Request, { params }: { params: Promise<{ slug: string[] }> }) {
    const { slug } = await params;
    const page = source.getPage(slug.slice(0, -1));
    if (!page) notFound();

    return new ImageResponse(
      <DefaultImage title={page.data.title} description={page.data.description} site={site.name} />,
      {
        width: 1200,
        height: 630,
      },
    );
  }

  function generateStaticParams() {
    return source.getPages().map((page) => ({
      slug: getPageImage(page).segments,
    }));
  }

  return { GET, generateStaticParams };
}
