import defaultMdxComponents from "fumadocs-ui/mdx";
import { Accordion, Accordions } from "fumadocs-ui/components/accordion";
import { Step, Steps } from "fumadocs-ui/components/steps";
import { Tab, Tabs } from "fumadocs-ui/components/tabs";
import type { MDXComponents } from "mdx/types";
import type { CrdChoices } from "../crd-form/choices";
import { CrdBuilder } from "../crd-form/CrdBuilder";
import { CrdForm } from "../crd-form/CrdForm";

/**
 * The components every operator's MDX can use: Fumadocs' defaults, the
 * accordion/steps/tabs blocks, and the manifest builder (`<CrdForm kind=… />`
 * on each generated CRD page, `<CrdBuilder />` for a kind picker in front of
 * it), bound to the operator's guided choices. The operator's `mdx.tsx`
 * spreads this and adds its own components (business diagrams…).
 */
export function docsMdxComponents(choices: CrdChoices = {}) {
  function BoundCrdForm({ kind }: { kind: string }) {
    return <CrdForm kind={kind} choices={choices[kind]} />;
  }
  function BoundCrdBuilder() {
    return <CrdBuilder choices={choices} />;
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
  } satisfies MDXComponents;
}
