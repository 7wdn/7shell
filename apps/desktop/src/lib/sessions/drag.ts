/**
 * A pointer over a row, turned into what dropping there would mean.
 *
 * The bands come from what the two kinds of row can offer. A group row has three: the ends mean
 * "beside it, at its parent" and the middle means "inside it". A session row has two halves, and
 * both mean "beside this session, in its parent" — which is also how a thing changes groups, since
 * the row it lands beside is drawn in the parent it will join. Because groups and sessions share
 * one order inside a parent, either can be dropped beside either: that is how the two interleave.
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

/** Which half of a session row the pointer is in: before the session or after it. */
export function halfIn(box: RowBox, pointerY: number): "before" | "after" {
  const height = Math.max(1, box.height);
  return pointerY - box.top < height / 2 ? "before" : "after";
}

/**
 * What dropping something on this row, at this height, would mean.
 *
 * A group row has three bands: the ends mean "beside it, at its parent" and the middle means
 * "inside it". A session row has two halves, and both mean "beside this session, in its parent" —
 * which is also how a thing changes groups, since the row it lands beside is drawn in the parent it
 * will join. Because groups and sessions share one order inside a parent, either can be dropped
 * beside either: that is how the two interleave.
 *
 * What comes back is a *wish*, not a decision: whether it can be done is the planner's answer
 * (`planGroupDrop` / `planSessionDrop`), and asking the same functions the drop itself will ask is
 * what keeps the indicator from promising something the release would refuse.
 */
export function dropAt(target: DragTarget, box: RowBox, pointerY: number): AimedDrop {
  if (target.kind === "group") {
    return { drop: { kind: "group", id: target.id }, zone: zoneIn(box, pointerY) };
  }
  return { drop: { kind: "session", id: target.id }, zone: halfIn(box, pointerY) };
}
