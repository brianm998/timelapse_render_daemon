# Task 1349: the Linux self-test creates the system menu directory

`Desktop self-test (linux-x64)` failed in `apt-get install` of the desktop `.deb`: dpkg reported
errors while processing `timelapse-render-monitor`, so the self-test never ran.

The cause is inferred, not reproduced: the log excerpt does not include the maintainer script's own
message, and no package was built or installed. The app sets a Linux icon, so jpackage generates a
`.desktop` file and a postinst that runs `xdg-desktop-menu install`. In system mode that command
exits non-zero ("No writable system menu directory found") when `/usr/share/desktop-directories`
is absent, as it may be on a runner with no desktop environment. The fix is unverified until the
GitHub run on `develop`.

- The workflow creates the directory before installing, so the runner looks like the desktop system
  the package targets. The launcher, self-test and removal checks are unchanged.
- Overriding jpackage's postinst through a resource directory was not chosen: it depends on the
  Compose plugin's internals. Dropping the Linux icon and menu entry would make the package worse
  for its users.
- The package is unchanged, so installing it on a Linux system with no desktop menu directory can
  still fail.
