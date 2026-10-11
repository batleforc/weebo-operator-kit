// Shared SVG styles for the docs' diagrams. Colours come from tokens.css,
// so every figure follows the light/dark rendition like the rest of the page.

export const box = { fill: 'var(--ground-sunk)', stroke: 'var(--rule-strong)', strokeWidth: 1 };
export const frame = {
  fill: 'none',
  stroke: 'var(--rule-soft)',
  strokeWidth: 1,
  strokeDasharray: '4 3',
};
export const ink = { fill: 'var(--ink)' };
export const dim = { fill: 'var(--ink-dim)' };
export const code = { fill: 'var(--copper)' };
export const wire = { stroke: 'var(--ink-dim)', strokeWidth: 1.25, fill: 'none' };
export const grant = { stroke: 'var(--accent)', strokeWidth: 1.5, fill: 'none' };

export const figureClass =
  'not-prose my-6 overflow-x-auto overscroll-x-contain border border-(--rule-soft) bg-(--ground)';
export const captionClass =
  'border-t border-(--rule-soft) px-3 py-2 text-[12px] text-fd-muted-foreground';
