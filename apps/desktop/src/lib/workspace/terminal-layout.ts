/** Layout nodes own geometry; terminal instances keep their handles and screens outside the tree. */
export type TerminalSplitDirection = "right" | "down";

export type TerminalLayout =
  | { readonly kind: "pane"; readonly paneId: string }
  | {
      readonly kind: "split";
      readonly id: string;
      readonly direction: TerminalSplitDirection;
      readonly ratio: number;
      readonly first: TerminalLayout;
      readonly second: TerminalLayout;
    };

export interface TerminalWorkspace {
  readonly id: string;
  layout: TerminalLayout;
  focusedId: string;
  zoomedId: string | null;
}

export interface PaneRect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

export interface TerminalDivider {
  readonly id: string;
  readonly direction: TerminalSplitDirection;
  readonly region: PaneRect;
  readonly rect: PaneRect;
  readonly ratio: number;
  readonly minRatio: number;
  readonly maxRatio: number;
}

export const TERMINAL_DIVIDER_PX = 8;
const MIN_TERMINAL_WIDTH = 240;
const MIN_TERMINAL_HEIGHT = 160;

export function terminalPaneIds(layout: TerminalLayout): string[] {
  return layout.kind === "pane"
    ? [layout.paneId]
    : [...terminalPaneIds(layout.first), ...terminalPaneIds(layout.second)];
}

export function splitTerminalPane(
  layout: TerminalLayout,
  targetId: string,
  paneId: string,
  direction: TerminalSplitDirection,
  splitId: string,
): TerminalLayout {
  if (terminalPaneIds(layout).includes(paneId)) {
    return layout;
  }
  function insert(node: TerminalLayout): TerminalLayout {
    if (node.kind === "pane") {
      return node.paneId === targetId
        ? { kind: "split", id: splitId, direction, ratio: 0.5, first: node, second: { kind: "pane", paneId } }
        : node;
    }
    return { ...node, first: insert(node.first), second: insert(node.second) };
  }
  return insert(layout);
}

/** Removing a pane collapses its parent, so no empty branch can strand the remaining terminal. */
export function removeTerminalPane(layout: TerminalLayout, paneId: string): TerminalLayout | null {
  if (layout.kind === "pane") {
    return layout.paneId === paneId ? null : layout;
  }
  const first = removeTerminalPane(layout.first, paneId);
  const second = removeTerminalPane(layout.second, paneId);
  return first === null ? second : second === null ? first : { ...layout, first, second };
}

export function resizeTerminalSplit(layout: TerminalLayout, splitId: string, ratio: number): TerminalLayout {
  if (layout.kind === "pane" || !Number.isFinite(ratio)) {
    return layout;
  }
  return {
    ...layout,
    ratio: layout.id === splitId ? Math.max(0.01, Math.min(0.99, ratio)) : layout.ratio,
    first: resizeTerminalSplit(layout.first, splitId, ratio),
    second: resizeTerminalSplit(layout.second, splitId, ratio),
  };
}

function minimum(layout: TerminalLayout): { width: number; height: number } {
  if (layout.kind === "pane") {
    return { width: MIN_TERMINAL_WIDTH, height: MIN_TERMINAL_HEIGHT };
  }
  const first = minimum(layout.first);
  const second = minimum(layout.second);
  return layout.direction === "right"
    ? { width: first.width + second.width + TERMINAL_DIVIDER_PX, height: Math.max(first.height, second.height) }
    : { width: Math.max(first.width, second.width), height: first.height + second.height + TERMINAL_DIVIDER_PX };
}

/** All pane hosts stay siblings; only these rectangles change when a tree is rearranged. */
export function terminalGeometry(
  layout: TerminalLayout,
  width: number,
  height: number,
): { panes: Record<string, PaneRect>; dividers: TerminalDivider[] } {
  const panes: Record<string, PaneRect> = {};
  const dividers: TerminalDivider[] = [];
  function visit(node: TerminalLayout, region: PaneRect): void {
    if (node.kind === "pane") {
      panes[node.paneId] = region;
      return;
    }
    const horizontal = node.direction === "right";
    const dimension = horizontal ? region.width : region.height;
    const divider = Math.min(TERMINAL_DIVIDER_PX, dimension);
    const available = dimension - divider;
    const firstMin = minimum(node.first)[horizontal ? "width" : "height"];
    const secondMin = minimum(node.second)[horizontal ? "width" : "height"];
    // In a small window the floors scale together, keeping every descendant visible. The stored
    // ratio remains untouched, so enlarging the window restores the layout the person chose.
    const scale = Math.min(1, available / (firstMin + secondMin));
    const low = firstMin * scale;
    const high = available - secondMin * scale;
    const firstSize = Math.max(low, Math.min(high, available * node.ratio));
    const secondSize = Math.max(0, available - firstSize);
    const first: PaneRect = { ...region, ...(horizontal ? { width: firstSize } : { height: firstSize }) };
    const rect: PaneRect = horizontal
      ? { ...region, left: region.left + firstSize, width: divider }
      : { ...region, top: region.top + firstSize, height: divider };
    const second: PaneRect = horizontal
      ? { ...region, left: rect.left + divider, width: secondSize }
      : { ...region, top: rect.top + divider, height: secondSize };
    dividers.push({
      id: node.id,
      direction: node.direction,
      region,
      rect,
      ratio: available > 0 ? firstSize / available : 0.5,
      minRatio: available > 0 ? low / available : 0.5,
      maxRatio: available > 0 ? high / available : 0.5,
    });
    visit(node.first, first);
    visit(node.second, second);
  }
  visit(layout, { left: 0, top: 0, width: Math.max(0, width), height: Math.max(0, height) });
  return { panes, dividers };
}
