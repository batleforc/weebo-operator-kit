"use client";

// The project as a graph: a view of the list's data, loaded only when asked
// for (it carries @xyflow/react). Every action has its equivalent in the
// list, which stays the way to work with the keyboard or a screen reader.

import { useMemo } from "react";
import { Background, Controls, Position, ReactFlow, type Edge, type Node } from "@xyflow/react";
import "@xyflow/react/dist/base.css";
import type { Graph } from "./model/graph";
import { layout } from "./model/layout";
import { formatPath } from "./model/paths";

const ORIGIN = { resource: "", produced: " (made)", external: " (external)" } as const;

export default function GraphView({
  graph,
  selected,
  onSelect,
}: {
  graph: Graph;
  selected: string | undefined;
  onSelect: (id: string) => void;
}) {
  // 240px nodes (global.css), with room for an edge's label between columns.
  const positions = useMemo(() => layout(graph, { x: 360, y: 100 }), [graph]);
  const edges = useMemo(
    (): Edge[] => [
      ...graph.slots.flatMap((slot, i): Edge[] =>
        slot.target ? [{ id: `ref-${i}`, source: slot.resource, target: slot.target.id, label: formatPath(slot.path) }] : [],
      ),
      ...graph.nodes.flatMap((n): Edge[] =>
        n.from ? [{ id: `made-${n.id}`, source: n.id, target: n.from, className: "crd-ide-edge-made", label: "made by" }] : [],
      ),
    ],
    [graph],
  );
  const nodes = useMemo(() => {
    return graph.nodes.map((n): Node => {
      const label = `${n.kind.kind} ${n.namespace ? `${n.namespace}/` : ""}${n.name}${ORIGIN[n.origin]}`;
      return {
        id: n.id,
        position: positions.get(n.id) ?? { x: 0, y: 0 },
        data: { label },
        ariaLabel: label,
        selected: n.id === selected || n.from === selected,
        // What is depended on sits on the left: references leave on the left.
        sourcePosition: Position.Left,
        targetPosition: Position.Right,
        className: `crd-ide-node crd-ide-node-${n.origin}`,
      };
    });
  }, [graph, positions, selected]);

  // A made object opens its maker; an external has no editor.
  const open = (id: string) => {
    const node = graph.nodes.find((n) => n.id === id);
    if (node?.origin === "resource") onSelect(node.id);
    else if (node?.from) onSelect(node.from);
  };

  return (
    <div className="crd-ide-graph h-[28rem] border border-(--rule-soft)" role="group" aria-label="Project graph, a view of the resource list below, which has every action">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        fitView
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        onNodeClick={(_, node) => open(node.id)}
        onSelectionChange={({ nodes: picked }) => {
          const [first] = picked;
          if (first && first.id !== selected) open(first.id);
        }}
      >
        <Background />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}
