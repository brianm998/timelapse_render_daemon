# Task 1330: SetConfigPath reaches the running engine

`SetConfigPath` used to replace only the server's config slot, so the engine kept reading the
path it started with and a fresh install stayed in `PHASE_NO_CONFIG` until a restart.

The server now stores the setting, rebuilds its slot, and sends the new path, the slot's codec
table and the chosen config's `render_history_path` to the engine before it answers. Concurrent
calls are serialised, so the server and the engine end on the same path. The engine switches at
once and ticks at once.

What a switch to a different path resets:

- the config store (a missing or invalid new file gives `PHASE_NO_CONFIG` naming the new path; the
  old config is not kept), the codec table and the unknown-codec problems;
- the last scan and the scanner, so the old config's backup candidates go and the new backup paths
  load at once;
- the Swift history path, but only while start-up has not run yet, so a fresh install imports the
  chosen config's history at its first loaded tick.

What it does not touch:

- running renders: nothing is signalled or stopped. Queued jobs the new config does not want go at
  the next scan, as for a config edit;
- start-up work (adoption, import, sweep) that already ran, and the stored history.

Setting the current path again only asks for a rescan, so the last good config stays until the
file reads again.

Not done: the desktop's "restart" hint is unchanged, and the codec table is still read once per
engine; both are filed as proposals.
