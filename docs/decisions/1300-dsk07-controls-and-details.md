# DSK-07: desktop controls and Details pane decisions

- "Pause all rendering asks the first time only, with the same checkbox": the confirmation's
  "Don't ask again" starts ticked for Pause all and clear for Stop, so Pause all is asked once unless
  the user clears the box. Cancel stores nothing.
- Pause all is confirmed wherever it is chosen (switch, menu, tray), in the main window, which is
  brought up for it. Switching servers drops an open confirmation.
- Variant buttons follow §6.4 literally: QUEUED has Stop only; FAILED has Pause variant and Retry;
  PAUSED has Resume variant and Retry.
- Variant icons are drawn on a canvas: no icon library is a dependency and glyphs depend on fonts.
- Double click is detected beside `selectable`, not with `combinedClickable`, which would delay
  the selecting click.
- The Details pane reads the raw snapshot messages, not the filtered presenter rows, so it shows
  every field and is not emptied by the search. `ActivityPresenter.renderedVideo` serves older pages.
- Render details are loaded on a change of target, of a variant's status or a failure's last
  occurrence and after a reconnect, not on progress; Refresh covers the growing log.
