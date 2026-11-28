# DSK-03: shared state layer decisions

- `ScrollKeeper` is stateful only in the running count of rows inserted above the anchor; it resets
  when the list is at the top or on `reset()`. When the anchor row vanishes it falls back to the
  nearest surviving neighbour (below first), and when none survives it returns `StayAtTop`: it never
  jumps to the bottom (S6).
- `ConfigEditorModel.canSave` is false while loading, saving, validating (an edit's validation has not
  answered) or while an ERROR issue exists. It does not require the copy to be dirty; the UI combines
  `dirty` itself. `save()` sends `UpdateConfig` first, then `UpdateCodecs` only when the codecs differ
  from those returned by the config update.
- `ThumbnailCache.get` returns null for NOT_FOUND (remembered for 10 minutes by the injected clock) and
  throws other failures without remembering them. Disk eviction orders by the clock-stamped file
  modification time, so tests are deterministic.
- `EventWatcher` stores each entry's `seq` before emitting, so a crash can lose a notification but
  never repeat one.
- `FakeTrdService` gained scripted validation results, per-id thumbnails and directories, gates that
  hold a handler (`GetThumbnail`, `RescanNow`), recorded request messages, and `ListLog` /
  `ListRenderedVideos` that honour paging parameters. Existing defaults are unchanged.
