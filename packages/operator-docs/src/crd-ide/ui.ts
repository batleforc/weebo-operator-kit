// The form classes of `crd-form`, shared by the IDE's views.

export const inputClass =
  "w-full border border-(--rule-strong) bg-(--ground-sunk) px-3 py-2 text-[15px] text-fd-foreground placeholder:text-fd-muted-foreground focus:border-(--accent)";

export const labelClass = "text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground";

export const panelClass = "flex min-w-0 flex-col gap-3 border border-(--rule-soft) p-3";

/** A panel's title: an h2 for the outline, without the page's h2 rule and spacing. */
export const headingClass = `${labelClass} m-0! border-0! p-0!`;

/** Saves `content` as a file named `name`. */
export function download(name: string, content: BlobPart, type: string): void {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}
