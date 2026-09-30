# Proposals (PIPs)

A **PIP** — a project improvement proposal — is a short document written *before* a change that
is expensive to get wrong. It exists so that the reasoning survives longer than the pull request
discussion, and so that a decision can be revisited later by reading why it was made.

## When to write one

Write a PIP when a change:

- touches an invariant in [../security-model.md](../security-model.md),
- changes a public contract (the error envelope, the wire types, the descriptor schema),
- changes the crate layout or the direction of a dependency,
- adds a dependency that is hard to remove, or a background process,
- changes how the project is released or what it promises about itself.

Do **not** write one for a bug fix, a new component, a new protocol descriptor, or anything that
can be reverted by reverting a commit. For those, a clear pull request description is the right
amount of process.

## How it works

1. Copy [`template.md`](template.md) to `docs/pips/PIP-NNNN-short-title.md`, taking the next
   number.
2. Open it as a pull request with the document alone, before writing the code. Its status starts
   as `Proposed`.
3. Discussion happens on that pull request. Edit the document rather than the thread.
4. When it is settled: `Accepted`, `Rejected`, or `Withdrawn`, with the reasoning recorded. An
   `Accepted` PIP that is implemented keeps its number forever, and code that contradicts it
   should cite it.

Planning notes that are larger than a PIP — dated plans, design explorations, handoff notes —
belong in a PIP's `plans/` folder when they are worth keeping, so that the decision and the
working notes stay together.

## Status values

| Status | Meaning |
|---|---|
| `Proposed` | Under discussion; nothing should be built on it yet |
| `Accepted` | Agreed, and the code may follow |
| `Rejected` | Decided against, with the reason recorded — this is the most valuable outcome to write down |
| `Withdrawn` | The author took it back |
| `Superseded` | Replaced; say by which number |
