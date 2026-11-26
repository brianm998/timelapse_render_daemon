# 1155 — Linux packages: unit enablement and what the tarball carries

Status: accepted (TRD-25)

Decision: `postinst` and `prerm` enable and disable the unit with `systemctl --global` only when
systemd is running (`command -v systemctl` and `/run/systemd/system` present). Without it they
print what to run later and exit 0, so `apt-get install` succeeds in a container.

Decision: a failing `systemctl` inside `postinst` or `prerm` prints a warning and exits 0. A
package that refuses to install or remove because a user manager is unavailable is worse than a
unit left for the admin to enable by hand.

Decision: the `.tar.gz` carries the `usr/lib/trd` tree, the user unit, the licence directory
`usr/share/doc/trd` and `INSTALL.txt`. It leaves out the `/usr/bin/trd` link, which `INSTALL.txt`
tells the installer to create. The licences ship with the binaries they cover, in both formats.

Rejected: copying `trd.service` verbatim. Its `@EXEC_START@` placeholder would give systemd an
empty `ExecStart`; the build renders it and fails if the placeholder survives.

Rejected: a systemd-free fallback that writes the unit under `/etc`. §10.2 names the packaged
path; the admin enables the unit with the command the install prints.
