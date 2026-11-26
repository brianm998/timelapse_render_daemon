# 1135 — t0-diff-check diffs against a real merge-base, never HEAD^

Status: accepted (TRD-1187)

Decision: t0-diff-check takes its base as the merge-base of HEAD with the first of
`origin/develop` then `develop` that resolves. The diff therefore covers the whole change, not
only HEAD.

Decision: there is no HEAD^ or HEAD-only fallback. When neither ref resolves, the suite exits 1
and names `origin/develop` in its message. A checkout without the base fails loudly instead of
passing a diff that checks one commit.

Decision: the operational fix for a checkout without `origin/develop` (node marmot) is for the
node to fetch `origin/develop`. The gate does not work around a missing ref.

Until node marmot's checkout carries origin/develop, `.mason/mason.toml` marks t0-diff-check
`remote = false`, as it does the iOS and Android suites, so the gate runs on the coordinator where
the ref exists. Drop that line once the node fetches origin/develop.
