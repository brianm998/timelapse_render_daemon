# 1130 — `.mason/**` no longer selects t0-mason in development

Status: accepted (TRD-1177)

Problem: task 1162 moved t0-mason to level t0-owner, bound to test_step and batch only. The impact
map entry `".mason/**" = ["t0-workflows", "t0-mason"]` still names it, and impact selection does
not apply the level binding. So every change under `.mason/` selects t0-mason in development, and
on a non-owner node (marmot) it runs and exits 1 because `mason` is not installed. The development
tests check stays red for every `.mason` change.

Options:
- Drop t0-mason from the `.mason/**` impact entry. Change made in this repository.
- Make impact respect level bindings. Change made in mason itself, not in this repository.

Decision: drop t0-mason from the `.mason/**` impact entry, in `.mason/mason.toml` and in the
matching block of `v3/1-trd-core-daemon.md` §11.3. The entry is now `["t0-workflows"]`.

Why this is enough: t0-mason stays in `[step.test]`'s suites list and keeps its `[suite.t0-mason]`
binding and command, so test_step still runs it on the owner's Mac, and batch still runs it there.
Only the impact selection for development changes.

Not taken: making impact respect bindings. It is the more general fix, but it lives in mason and
is not something this repository can land. It can be revisited there later; this record does not
depend on it.
