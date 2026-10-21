import { docsMdxComponents } from '@weebo/operator-docs/site';
import type { MDXComponents } from 'mdx/types';
import { crdSources } from '@/lib/catalog';

// The kit has no CRD of its own: no guided choices for the manifest
// builder; the CRD IDE offers the catalog's (crd-catalog.config.mjs).
export function getMDXComponents(components?: MDXComponents) {
  return {
    ...docsMdxComponents({}, { sources: crdSources }),
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
