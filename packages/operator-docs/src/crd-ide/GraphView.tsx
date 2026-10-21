"use client";

// The project as a graph: a view of the list's data, loaded only when asked
// for (it carries @xyflow/react). Every action has its equivalent in the
// list, which stays the way to work with the keyboard or a screen reader.

import { useEffect, useMemo, useState } from "react";
import { applyNodeChanges, Background, Controls, Position, ReactFlow, type Edge, type Node } from "@xyflow/react";
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
  // Labels on the selected resource's edges only: on every edge, they pile
  // up along the columns.
  const edges = useMemo((): Edge[] => {
    const labelled = (a: string, b: string, label: string) =>
      selected !== undefined && (a === selected || b === selected) ? { label } : {};
    return [
      ...graph.slots.flatMap((slot, i): Edge[] =>
        slot.target
          ? [{ id: `ref-${i}`, source: slot.resource, target: slot.target.id, ...labelled(slot.resource, slot.target.from ?? slot.target.id, formatPath(slot.path)) }]
          : [],
      ),
      ...graph.nodes.flatMap((n): Edge[] =>
        n.from ? [{ id: `made-${n.id}`, source: n.id, target: n.from, className: "crd-ide-edge-made", ...labelled(n.from, n.from, "made by") }] : [],
      ),
    ];
  }, [graph, selected]);
  const nodes = useMemo(() => {
    return graph.nodes.map((n): Node => {
      const label = `${n.kind.kind} ${n.namespace ? `${n.namespace}/` : ""}${n.name}${ORIGIN[n.origin]}`;
      return {
        id: n.id,
        position: positions.get(n.id) ?? { x: 0, y: 0 },
        data: { label },
        ariaLabel: label,
        // What is depended on sits on the left: references leave on the left.
        sourcePosition: Position.Left,
        targetPosition: Position.Right,
        // The current resource (and what it makes) is marked by a class, not
        // xyflow's `selected`: the IDE's selection and xyflow's would answer
        // each other without end.
        className: `crd-ide-node crd-ide-node-${n.origin}${n.id === selected || n.from === selected ? " crd-ide-current" : ""}`,
      };
    });
  }, [graph, positions, selected]);

  // xyflow measures the nodes through onNodesChange: without it, nothing
  // is measured and `fitView` never fits.
  const [flowNodes, setFlowNodes] = useState(nodes);
  useEffect(() => setFlowNodes(nodes), [nodes]);

  // A made object opens its maker; an external has no editor.
  const open = (id: string) => {
    const node = graph.nodes.find((n) => n.id === id);
    if (node?.origin === "resource") onSelect(node.id);
    else if (node?.from) onSelect(node.from);
  };

  return (
    <div className="crd-ide-graph h-[28rem] border border-(--rule-soft)" role="group" aria-label="Project graph, a view of the resource list below, which has every action">
      <ReactFlow
        // A new set of nodes is fitted again.
        key={graph.nodes.length}
        nodes={flowNodes}
        edges={edges}
        onNodesChange={(changes) => {
          setFlowNodes((current) => applyNodeChanges(changes, current));
          // A node picked with the mouse or the keyboard opens its resource.
          const picked = changes.find((c) => c.type === "select" && c.selected);
          if (picked?.type === "select") open(picked.id);
        }}
        fitView
        fitViewOptions={{ padding: 0.1 }}
        // A chain of a dozen columns still fits the docs' width.
        minZoom={0.15}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        onNodeClick={(_, node) => open(node.id)}
      >
        <Background />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}
