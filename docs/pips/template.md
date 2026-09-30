# PIP-NNNN: Title

- **Status:** Proposed
- **Date:** YYYY-MM-DD
- **Affects:** the crates, files or contracts this changes

## Summary

One paragraph. What is being proposed, in the terms someone would use to describe it to a
colleague, so that a reader can decide in ten seconds whether they need to read the rest.

## Background

What exists today, and what is wrong with it. Facts, with file paths — not opinions yet. If there
is a measurement behind the proposal, put it here.

## Proposal

What would change, concretely enough to be argued with: which crates, which files, which
contracts, which user-visible behaviour. Include what does *not* change, because that is usually
what the disagreement is about.

## Alternatives considered

The options that were rejected, each with the reason. "Do nothing" is always an alternative and
should be stated with its real cost. A proposal with no alternatives reads as a conclusion looking
for justification.

## Impact

| Area | Effect |
|---|---|
| Security invariants | Which of the rules in `docs/security-model.md` this touches, and how they stay true |
| Wire format / contracts | Anything the interface or a future caller has to change with it |
| Tests | What proves the change works, and what proves it did not break something |
| Migration | What happens to data, settings or saved state written by the current version |
| Dependencies | Anything new, and why the existing set is not enough |

## Open questions

What is genuinely undecided. Listing these is not weakness; it is what makes the discussion land
on the parts that matter.
