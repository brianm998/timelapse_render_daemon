# Task 1346: the e2e test holds the renders it controls

`e2e_daemon_with_real_tools` suspends, resumes and re-adopts renders of the real ffmpeg. A sandbox
render (16 to 60 frames of 320x213) can end in under a second, while the snapshot that showed it
running is published after the rest of the engine tick and an ended process is only reaped on the
next one. SuspendRender then correctly answered `FAILED_PRECONDITION` and the test failed. This
cause was found by reading the code; no failing run was reproduced.

- The daemon under test gets a `/bin/sh` stand-in as `ffmpeg` (in a temporary `TRD_FFMPEG_DIR`). It
  holds the renders the test names until the test releases them, then `exec`s the real ffmpeg with
  the same pid, start time and arguments. Every render is still made by the real ffmpeg.
- Larger frames, `-readrate` or retrying the controls were not used: each still depends on the speed
  of the machine or on the ffmpeg version, and retries would weaken the assertions.
- The trade-off: in this test, suspend, resume, the daemon crash and adoption act on
  the process before it has become ffmpeg. The same controls on a process in mid-render are covered
  by the tests that use the fake ffmpeg.
- No product code changed.
