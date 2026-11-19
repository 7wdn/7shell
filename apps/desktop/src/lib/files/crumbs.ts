/**
 * Which breadcrumb starts the visible run when a path is wider than its row.
 *
 * The folders nearest the current one are the ones worth keeping: they say where somebody is.
 * The leading ones fold into an ellipsis, and the last crumb is always kept even when it has to be
 * truncated itself. Widths are the measured crumb buttons, so the answer follows the interface's
 * own font and scaling rather than a guess at character counts.
 */
const GAP_PX = 2;
const CHEVRON_PX = 11;
const ELLIPSIS_PX = 20;

export function firstVisibleCrumb(widths: readonly number[], available: number): number {
  if (widths.length === 0) {
    return 0;
  }

  const all = widths.reduce((total, width, index) => total + width + (index > 0 ? CHEVRON_PX + GAP_PX * 2 : 0), 0);
  if (all <= available) {
    return 0;
  }

  let first = widths.length - 1;
  let kept = widths[first] ?? 0;
  const budget = available - ELLIPSIS_PX - CHEVRON_PX - GAP_PX * 2;
  for (let index = widths.length - 2; index >= 0; index -= 1) {
    const next = (widths[index] ?? 0) + CHEVRON_PX + GAP_PX * 2;
    if (kept + next > budget) {
      break;
    }
    kept += next;
    first = index;
  }
  return first;
}
