# DSK-06: desktop Activity view decisions

- Each variant is a list item of its own, so every overview §6.2 S4 key anchors the scroll position.
  Item keys carry their kind (`card:<id>`, `video:<id>`), because ids of different kinds may be equal.
- "At the top" is first visible item 0 at offset 0, as spec 2 §6.3 says.
- Section headers, empty texts and "Show older" are list items but not rows: `ScrollKeeper.decide`
  takes a predicate so the pill does not count them.
- Rows that appear because the search or export-path filter changed are not counted as new.
  `ActivityUi` carries the filter it was built with so the list can tell.
- Posters are cached under the platform's cache directory (`defaultCacheDir`), one directory per
  server id as the server reports it, so a sandbox `trd` and the installed one do not share posters.
- A click selects a row and highlights it. Opening a video on double click is a row action and comes
  with the other actions (DSK-07), as do the card menu, the variant and failure buttons.
- File names are truncated in the middle by our own composable; their semantics keep the whole name.
- A poster that cannot be fetched or decoded stays a placeholder, is reported on stderr with the
  video id and is asked for again after a reconnect. Undecodable bytes are dropped from the cache,
  which would otherwise answer every retry with them. A video without a poster is not reported.
