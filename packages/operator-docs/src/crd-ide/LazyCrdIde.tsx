"use client";

// The IDE's code loads only on the page that shows it: the MDX components
// every page shares hold this wrapper, not the IDE.

import dynamic from "next/dynamic";

export const LazyCrdIde = dynamic(() => import("./CrdIde").then((m) => m.CrdIde), {
  ssr: false,
  loading: () => <p data-crd-ide="" className="border border-(--rule-strong) px-6 py-10 text-[13px] text-fd-muted-foreground">Loading the IDE...</p>,
});
