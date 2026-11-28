# DSK-05: desktop shell decisions

- The not-running screen is for a local server that has not answered since it was selected. A
  connection lost later keeps the main window with the client's offline text, so the rows last seen
  stay visible. Until the first attempt is answered the window says "Not connected".
- The local server's 2 s probe is the client's own connect loop with the back-off replaced; the gRPC
  channel's back-off is reset after every wait so each attempt dials.
- Menu items whose windows come later (Pair a phone…, Config…, Log…, Preferences…) are present and
  disabled. Loopback-only items are disabled, not hidden, on other connections.
- Pairing a discovered server (code entry) and forgetting a paired one (overview §6.6) are part of
  the shell, since no later desktop ticket owns them.
- `--server` accepts loopback hosts only, starts on the local entry, and does not overwrite the
  remembered server by itself.
- The menus are a plain model (`menuModel`) drawn by `AppMenuBar`, so they are tested without
  opening a window.
- The macOS user id for `launchctl` comes from `/usr/bin/id -u` through the same command runner,
  which avoids adding a JDK module to the packaged runtime.
