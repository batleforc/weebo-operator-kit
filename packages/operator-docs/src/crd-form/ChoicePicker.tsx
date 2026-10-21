"use client";

import type { KeyboardEvent } from "react";
import type { ChoiceGroup, Selection } from "./choices";

// Arrow keys move focus and selection (wrapping), Home/End jump to the ends:
// the WAI-ARIA radio group pattern. Space/Enter select through the button's click.
function nextIndex(key: string, index: number, count: number): number | null {
  switch (key) {
    case "ArrowLeft":
    case "ArrowUp":
      return (index - 1 + count) % count;
    case "ArrowRight":
    case "ArrowDown":
      return (index + 1) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

// A three-or-fewer-way choice drawn the design system's way: one 1px bounding box,
// hairline cell dividers, no gaps, the chosen cell reversed to an ink block,
// state on `aria-checked` of a radio group (DESIGN.md, "Preferences" / segmented groups).
export function ChoicePicker({
  groups,
  selection,
  onChange,
}: {
  groups: ChoiceGroup[];
  selection: Selection;
  onChange: (groupId: string, optionId: string) => void;
}) {
  return (
    <div className="flex flex-col gap-4 border border-(--rule-soft) p-3">
      {groups.map((group) => {
        const current = group.options.find((o) => o.id === selection[group.id]) ?? group.options[0];
        if (!current) return null;
        const labelId = `choice-${group.id}`;
        return (
          <div key={group.id} className="flex flex-col gap-2">
            <span id={labelId} className="text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground">
              {group.label}
            </span>
            <div role="radiogroup" aria-labelledby={labelId} className="flex border border-(--rule-strong)">
              {group.options.map((option, index) => {
                const pressed = option.id === current.id;
                const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
                  const next = nextIndex(e.key, index, group.options.length);
                  const target = next === null ? undefined : group.options[next];
                  if (next === null || !target) return;
                  e.preventDefault();
                  onChange(group.id, target.id);
                  const button = e.currentTarget.parentElement?.children[next];
                  if (button instanceof HTMLElement) button.focus();
                };
                return (
                  <button
                    key={option.id}
                    type="button"
                    role="radio"
                    aria-checked={pressed}
                    tabIndex={pressed ? 0 : -1}
                    onClick={() => onChange(group.id, option.id)}
                    onKeyDown={onKeyDown}
                    className={`flex-1 px-1 py-2 text-[12px] uppercase tracking-[0.06em] not-first:border-l not-first:border-(--rule-strong) ${
                      pressed
                        ? "bg-(--ink) font-bold text-(--ground)"
                        : "text-fd-muted-foreground hover:text-fd-foreground"
                    }`}
                  >
                    {option.label}
                  </button>
                );
              })}
            </div>
            <p className="text-[13px] leading-[1.6] text-fd-muted-foreground">{current.hint}</p>
          </div>
        );
      })}
    </div>
  );
}
