# 1135 — t0-diff-check diffs against a real merge-base, never HEAD^

Status: accepted (TRD-1187)

Decision: t0-diff-check takes its base as the merge-base of HEAD with the first of
`origin/develop` then `develop` that resolves. The diff therefore covers the whole change, not
only HEAD.

Decision: there is no first-parent or HEAD-only fallback. When neither ref resolves, the suite
exits 1 and names `origin/develop` in its message. A checkout without the base fails loudly
instead of passing a diff that checks one commit.

Decision (owner): node marmot's checkout, which has no `origin/develop`, gets the ref by an
operator fetching `origin/develop` into it. That is an operational change on the node, outside
this repository; the repository keeps the loud failure.

Rejected: guessing a base from other refs on the node (e.g. MASON's per-task node refs). Nothing
guarantees a commit those refs share has landed on develop (stacked tasks share unlanded
commits), so the guessed base can be newer than the real merge-base and weaken the gate.

Rejected: `remote = false` for t0-diff-check. It hides the node's missing ref.
