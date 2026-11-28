# DSK-11: tray, notifications and preferences decisions

- The app watches the events of the active server only, the one it is connected to. Switching back
  to a server resumes from the last `seq` stored for it, so events that happened meanwhile are
  notified then. A Log window's own connection to another server (DSK-09) is not watched.
- The last `seq` is keyed by the server id that `GetServerInfo` reports, not the registry id, so a
  `--server` sandbox and the installed `trd` (both the local entry) keep their own.
- A log entry carries no kind label. It is taken from the failure card (waited for up to 2 s in the
  watched snapshots, then asked with `GetSnapshot`), else from the `FAILURE_*` code that starts the
  message, else "unknown failure". The code labels repeat those of trd's `diagnosis.rs`.
- Without a system tray, closing the window quits: a hidden window could not be opened again.
- The tray icon is drawn at run time (there is no image asset); its dot follows the unfiltered
  snapshot, and failures win over rendering.
- The interface is `AppPreferences`, to avoid the name of `java.util.prefs.Preferences`. The two
  "Don't ask again" values are stored here for the confirmation dialogs to use.
