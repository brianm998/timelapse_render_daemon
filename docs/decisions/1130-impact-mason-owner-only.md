# 1130 — `.mason/**` keeps selecting t0-mason (change tried and reverted)

Status: decided by the implementing agent on the owner's note (TRD-1177). The first attempt
dropped t0-mason from the `.mason/**` impact entry; that change is reverted here. The impact map
is as it was before task 1162's follow-up.

Problem as reported: task 1162 moved t0-mason to a level bound to test_step and batch only. The
impact map entry `".mason/**" = ["t0-workflows", "t0-mason"]` still names it, and impact selection
does not apply the level binding, so a change under `.mason/` selects t0-mason in development. On
the non-owner node marmot it then exited 1 because `mason` was not installed.

What changed since that report: the owner fixed masond in e509dc6 (live on marmot). Node jobs now
get `origin/<base>`, and `mason` is on the node's PATH, so t0-mason can run on a node. The
"mason not installed" exit 1 that the impact change was meant to avoid should no longer occur. The
owner's note asks for the change to be kept minimal and says it may not be needed. I have not been
able to run the suite on marmot from this run, so the claim that the failure is gone rests on the
owner's report of e509dc6, not on a run here.

Also, the `[binding.t0]` block in this checkout's `.mason/mason.toml` binds `t0` to
`development`, `test_step` and `batch`; `[suite.t0-mason]` is level `t0`. The owner-only level
described in the task text is not in this base, so there is no owner-only binding for the impact
entry to contradict here.

Decision: revert. Keep `".mason/**" = ["t0-workflows", "t0-mason"]` in `.mason/mason.toml` and in
the matching block of `v3/1-trd-core-daemon.md` §11.3, so the spec and the config still match. The
change had only one purpose, avoiding the marmot failure, and that purpose is gone. Reverting is the
smallest change, and it leaves nothing to undo if the owner later wants the entry narrowed.

Not taken: dropping t0-mason from the impact entry (the first attempt). Also not taken: making
impact respect level bindings in mason. That is a change in mason itself, not in this repository,
and nothing here needs it now.

Reopen if: marmot still exits 1 on t0-mason for a `.mason/` change after e509dc6, or the owner
moves t0-mason to an owner-only level in this repository. Then drop t0-mason from the `.mason/**`
entry in both files, as the first attempt did.
