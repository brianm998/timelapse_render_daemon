# DSK-10: Devices dialog and first-run setup decisions

- The Full Disk Access problem banner of DSK-06 carries **How to fix**
  (`AppState.openFullDiskAccessHelp`), over loopback on a Mac only.
- `trd` ranks "waiting for disk access" above "no config", so on a fresh Mac without access setup
  does not open by itself; **How to fix** opens it, and the config step appears once the phase
  becomes `PHASE_NO_CONFIG`.
- `SetConfigPath` stores the path, but the running engine reads it only after a restart. Setup says
  so while the snapshot still reports no config.
- A hand-entered server is typed with the 16 fingerprint digits that the pairing dialog shows (as on
  the phones), so the full certificate fingerprint is probed before pairing.
- Not verified by any test: the manual check M-DSK-8 (the real Privacy pane, Finder selection and
  the step completing by itself) and the native file chooser, which no test opens.
