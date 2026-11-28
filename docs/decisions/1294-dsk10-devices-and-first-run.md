# DSK-10: Devices dialog and first-run setup decisions

- The problem banners belong to DSK-06. Until they exist, the Activity placeholder shows the Full
  Disk Access problem with **How to fix** (`AppState.openFullDiskAccessHelp`), which the banner
  should call.
- `trd` ranks "waiting for disk access" above "no config", so on a fresh Mac without access setup
  does not open by itself; **How to fix** opens it, and the config step appears once the phase
  becomes `PHASE_NO_CONFIG`.
- `SetConfigPath` stores the path, but the running engine reads it only after a restart. Setup says
  so while the snapshot still reports no config.
- A hand-entered server is typed with the 16 fingerprint digits that the pairing dialog shows (as on
  the phones), so the full certificate fingerprint is probed before pairing.
