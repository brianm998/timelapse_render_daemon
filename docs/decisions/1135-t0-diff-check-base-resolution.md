# 1135 — t0-diff-check diffs against a real merge-base, never HEAD^

Status: accepted (TRD-1187)

Decision: t0-diff-check takes its base as the merge-base of HEAD with the first of
`origin/develop` then `develop` that resolves. The diff therefore covers the whole change, not
only HEAD.

Decision: there is no HEAD^ or HEAD-only fallback. When neither ref resolves, the suite exits 1
and names `origin/develop` in its message. A checkout without the base fails loudly instead of
passing a diff that checks one commit.

Decision: node marmot's checkout has no remotes and no `develop`; it is a MASON mirror that holds
the full history but only `refs/mason/node/<task>r<run>-...` refs. When neither ref resolves, the
base is the newest first-parent ancestor of HEAD that node refs of two or more distinct tasks
contain. A commit two tasks carry has landed on develop, and a task's own commits are carried only
by its own refs, so this base is never newer than the real merge-base: the diff covers at least
the whole change. If no such ancestor exists the suite still exits 1. The node fetching
`origin/develop` (the owner's chosen operational fix) makes the first branch apply again.

Rejected: `remote = false` for t0-diff-check. Placement is read from develop's definition, so it
would not take effect for the change that adds it, and it hides the node's missing ref.
