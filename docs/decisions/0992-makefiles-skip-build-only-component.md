# 0992: check-makefiles.sh skips a build-only component directory

## Root cause

`v3/scripts/suite.sh` (lines 32-41) is the junit placeholder: when a Kotlin suite runs before the
Kotlin client exists, it writes `v3/clients/kotlin/build/mason-{desktop,android}-results.xml`.
That creates `v3/clients/kotlin` with no Makefile, and `check-makefiles.sh` then reports
`v3/clients/kotlin: no Makefile` and fails `t0-makefiles` and `t0-scripts` for every ticket whose
worktree holds that directory. The build output is git-ignored by `v3/.gitignore`, so it is never
in git, and the directory is not a component yet.

## Decisions

- `check-makefiles.sh` treats a component directory whose only entry is a `build/` directory as not
  yet created and skips it. Any other entry (a stray file, a second directory, a dotfile) or an
  empty directory still needs a Makefile. The existing `v3/ios: no Makefile` test for an empty
  directory is unchanged.
- The exemption lives in the checker, not in the placeholder. The `results_file` paths in
  `.mason/mason.toml` are fixed, so moving the placeholder output elsewhere would change the mason
  contract for every ticket.
- Detection is by directory contents, not by git. Test fixtures are plain temporary directories
  with no repository, so a `git ls-files` check cannot run there.
- The root `v3/` directory is not exempted: `V3_ROOT` must still have a Makefile.

## Follow-up

Spec 1 (`v3/1-trd-core-daemon.md`, around line 1424) should mention this exemption next to the
component list in the Makefile check. This ticket does not edit the spec.
