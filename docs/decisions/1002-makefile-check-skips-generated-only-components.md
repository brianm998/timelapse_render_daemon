# 1002: the Makefile check skips generated-only component directories

## Decision

`v3/scripts/check-makefiles.sh` treats a component directory as absent when it has no Makefile
and its only entries are `build/` or `.gradle/`. Such a directory is skipped; any other directory
without a Makefile still fails with `v3/<comp>: no Makefile`.

This extends 0992, which skipped a directory whose only entry is `build/`, to also cover `.gradle/`.

## Why

`suite.sh kotlin-test` and `android-test` write placeholder junit files to
`clients/kotlin/build/` when the Kotlin client is absent. `.mason/mason.toml` reads their results
from exactly those paths, so moving the placeholders would break the suites. In a worktree that
lacks the client, `clients/kotlin` then holds only that generated output, and the Makefile check
failed on it.

## What still fails

- An empty directory without a Makefile (the `v3/ios` case).
- A directory with a source file or any other entry, such as `clients/kotlin/src/Foo.kt`,
  without a Makefile. A real component is never hidden by generated output beside it.
