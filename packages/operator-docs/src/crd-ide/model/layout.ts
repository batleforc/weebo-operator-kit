// Where the graph view draws each node: a layer per dependency depth, left
// to right (what is depended on first), in project order within a layer.
// Recomputed, never saved: the YAML is the project.

import { dependencies, type Graph } from "./graph";

export interface Position {
  x: number;
  y: number;
}

export function layout(graph: Graph, spacing = { x: 280, y: 120 }): Map<string, Position> {
  const deps = dependencies(graph);
  const depth = new Map<string, number>();
  const visiting = new Set<string>();
  const depthOf = (id: string): number => {
    const known = depth.get(id);
    if (known !== undefined) return known;
    // A cycle is cut where it is found.
    if (visiting.has(id)) return 0;
    visiting.add(id);
    const d = Math.max(-1, ...[...(deps.get(id) ?? [])].map(depthOf)) + 1;
    visiting.delete(id);
    depth.set(id, d);
    return d;
  };
  const rows = new Map<number, number>();
  const positions = new Map<string, Position>();
  for (const node of graph.nodes) {
    const d = depthOf(node.id);
    const row = rows.get(d) ?? 0;
    rows.set(d, row + 1);
    // Every other column sits half a row lower: an edge that skips columns
    // (to the instance everything lives on) does not run over the nodes
    // between.
    positions.set(node.id, { x: d * spacing.x, y: (row + (d % 2) / 2) * spacing.y });
  }
  return positions;
}
