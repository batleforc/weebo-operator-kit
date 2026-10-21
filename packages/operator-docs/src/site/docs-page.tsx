import type { ComponentProps, ReactNode } from "react";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import type { LoaderConfig, LoaderOutput, Page } from "fumadocs-core/source";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
  MarkdownCopyButton,
  ViewOptionsPopover,
} from "fumadocs-ui/layouts/docs/page";
import { createRelativeLink } from "fumadocs-ui/mdx";
import type { MDXComponents } from "mdx/types";
import type { SiteConfig } from "./config";
import { getPageImage, getPageMarkdownUrl } from "./source";

/** What a docs page needs from the operator's collection (fumadocs-mdx's `defineDocs`). */
interface MdxPageData {
  title?: string | undefined;
  description?: string | undefined;
  full?: boolean | undefined;
  body: (props: { components?: MDXComponents }) => ReactNode;
  toc: NonNullable<ComponentProps<typeof DocsPage>["toc"]>;
}

type MdxLoaderConfig = LoaderConfig & {
  page: Page<string | undefined, LoaderConfig["page"]["data"] & MdxPageData>;
};

interface DocsPageProps {
  params: Promise<{ slug?: string[] }>;
}

/**
 * The `app/docs/[[...slug]]/page.tsx` of every operator:
 *
 *   const docsPage = createDocsPage({ source, site, getMDXComponents });
 *   export default docsPage.Page;
 *   export const { generateStaticParams, generateMetadata } = docsPage;
 */
export function createDocsPage<C extends MdxLoaderConfig>({
  source,
  site,
  getMDXComponents,
}: {
  source: LoaderOutput<C>;
  site: SiteConfig;
  getMDXComponents: (components?: MDXComponents) => MDXComponents;
}) {
  async function Page(props: DocsPageProps) {
    const params = await props.params;
    const page = source.getPage(params.slug);
    if (!page) notFound();

    const MDX = page.data.body;
    const markdownUrl = getPageMarkdownUrl(page).url;
    const { user, repo, branch, contentDir } = site.git;

    return (
      // `full` turns the side TOC off in fumadocs-ui; a CRD page is full for its
      // form, and long enough to need the TOC. A page without a heading (the
      // CRD IDE's) has none: its column goes to the content.
      <DocsPage toc={page.data.toc} full={page.data.full} tableOfContent={{ enabled: page.data.toc.length > 0 }}>
        <DocsTitle>{page.data.title}</DocsTitle>
        <DocsDescription className="mb-0">{page.data.description}</DocsDescription>
        <div className="flex flex-row gap-2 items-center border-b pb-6">
          <MarkdownCopyButton markdownUrl={markdownUrl} />
          <ViewOptionsPopover
            markdownUrl={markdownUrl}
            githubUrl={`https://github.com/${user}/${repo}/blob/${branch}/${contentDir}/${page.path}`}
          />
        </div>
        <DocsBody>
          <MDX
            components={getMDXComponents({
              // this allows you to link to other pages with relative file paths
              a: createRelativeLink(source, page),
            })}
          />
        </DocsBody>
      </DocsPage>
    );
  }

  async function generateStaticParams() {
    return source.generateParams();
  }

  async function generateMetadata(props: DocsPageProps): Promise<Metadata> {
    const params = await props.params;
    const page = source.getPage(params.slug);
    if (!page) notFound();

    return {
      title: page.data.title,
      description: page.data.description,
      openGraph: {
        images: getPageImage(page).url,
      },
    };
  }

  return { Page, generateStaticParams, generateMetadata };
}
