/**
 * A pointer over a row, turned into what dropping there would mean.
 *
 * The bands come from what the two things that can be dragged have in common. A group dropped in
 * the middle of another group goes *inside* it, and dropped at either end goes beside it. A session
 * has no bands: it is in a group rather than between two things — our model keeps no order among
 * the sessions of one group — so the whole row means "the group this row is drawn in".
 *
 * What comes back is a *wish*, not a decision: whether it can be done is the planner's answer
 * (`planGroupDrop` / `planSessionDrop`), and asking the same functions the drop itself will ask is
 * what keeps the indicator from promising something the release would refuse.
 */
import type { Drop, DropZone } from "./groups";
import type { DragTarget } from "./tree";

/** What is being dragged: the two things a row can be. */
export type Grabbed =
  | { readonly kind: "group"; readonly id: string }
  | { readonly kind: "session"; readonly id: string };

/** A row's place on screen, which is all the banding needs. */
export interface RowBox {
  readonly top: number;
  readonly height: number;
}

/** Where a drop would land, and which band of the row was hit to say so. */
export interface AimedDrop {
  readonly drop: Drop;
  readonly zone: DropZone;
}

/** The share of a group row that counts as the group itself; the top and bottom are the sides. */
const SIDE_BAND = 0.28;

/** Which band of a group row the pointer is in. */
export function zoneIn(box: RowBox, pointerY: number): DropZone {
  const height = Math.max(1, box.height);
  const offset = (pointerY - box.top) / height;
  if (offset < SIDE_BAND) {
    return "before";
  }
  if (offset > 1 - SIDE_BAND) {
    return "after";
  }
  return "inside";
}

/** What dropping a group on this row, at this height, would mean. */
export function groupDropFor(target: DragTarget, box: RowBox, pointerY: number): AimedDrop {
  if (target.kind === "group") {
    return { drop: { kind: "group", id: target.id }, zone: zoneIn(box, pointerY) };
  }
  // Onto a session: the group goes where that row is — inside the group the row is drawn in, or at
  // the root when the row is in no group.
  return target.parentId === null
    ? { drop: { kind: "root" }, zone: "inside" }
    : { drop: { kind: "group", id: target.parentId }, zone: "inside" };
}

/** What dropping a session on this row would mean. A session only ever changes which group it is in. */
export function sessionDropFor(target: DragTarget): AimedDrop {
  if (target.kind === "group") {
    return { drop: { kind: "group", id: target.id }, zone: "inside" };
  }
  return target.parentId === null
    ? { drop: { kind: "root" }, zone: "inside" }
    : { drop: { kind: "group", id: target.parentId }, zone: "inside" };
}
