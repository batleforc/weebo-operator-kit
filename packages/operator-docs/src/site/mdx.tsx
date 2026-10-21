import defaultMdxComponents from "fumadocs-ui/mdx";
import { Accordion, Accordions } from "fumadocs-ui/components/accordion";
import { Step, Steps } from "fumadocs-ui/components/steps";
import { Tab, Tabs } from "fumadocs-ui/components/tabs";
import type { MDXComponents } from "mdx/types";
import type { CrdChoices } from "../crd-form/choices";
import { CrdBuilder } from "../crd-form/CrdBuilder";
import { CrdForm } from "../crd-form/CrdForm";
import type { Recipe } from "../crd-ide/CrdIde";
import { LazyCrdIde } from "../crd-ide/LazyCrdIde";
import type { CatalogSource } from "../crd-ide/load";

/** Where the CRD IDE finds its CRDs. */
export interface CrdIdeOptions {
  /** Schema directories; default the site's `/crd-schemas`. */
  sources?: CatalogSource[];
}

/**
 * The components every operator's MDX can use: Fumadocs' defaults, the
 * accordion/steps/tabs blocks, the manifest builder (`<CrdForm kind=… />`
 * on each generated CRD page, `<CrdBuilder />` for a kind picker in front of
 * it) and the CRD IDE (`<CrdIde />`, loaded only where used), bound to the
 * operator's guided choices. The operator's `mdx.tsx` spreads this and adds
 * its own components (business diagrams…).
 */
export function docsMdxComponents(choices: CrdChoices = {}, ide: CrdIdeOptions = {}) {
  function BoundCrdForm({ kind }: { kind: string }) {
    return <CrdForm kind={kind} choices={choices[kind]} />;
  }
  function BoundCrdBuilder() {
    return <CrdBuilder choices={choices} />;
  }
  // Recipes come from the page (`<CrdIde recipes={RECIPES} />`, imported in
  // its MDX): held here, their YAML would ship with every page.
  function BoundCrdIde({ recipes }: { recipes?: Recipe[] }) {
    return <LazyCrdIde choices={choices} recipes={recipes} sources={ide.sources} />;
  }
  return {
    ...defaultMdxComponents,
    Accordion,
    Accordions,
    Step,
    Steps,
    Tab,
    Tabs,
    CrdForm: BoundCrdForm,
    CrdBuilder: BoundCrdBuilder,
    CrdIde: BoundCrdIde,
  } satisfies MDXComponents;
}
