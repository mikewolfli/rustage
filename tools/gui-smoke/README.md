# GUI smoke (`tools/gui-smoke`)

Real-window evidence for the Linux GTK host. It is not part of `cargo test`: it needs an X11
session, GTK3, `xwininfo`, `xwd`, and `libXtst`.

```text
cargo build --features linux-native --bin rustage
python3 tools/gui-smoke/run.py
python3 tools/gui-smoke/run.py --keys
python3 tools/gui-smoke/run.py --ime    # needs a running fcitx5 with pinyin
```

What it proves:

- a real 1280×800 GTK window is mapped (found by `xwininfo`, not by the host's own log); the
  harness matches the child's `_NET_WM_PID`, so a stale window from an aborted run is never
  selected and never receives the synthetic input;
- the window paints two real `Button`s and a `LineEdit` (`smoke controls`); `xwd` shows the button
  rectangle has many distinct pixel values, not the single clear colour of an empty window;
- one XTest pointer click reaches the primary button and toggles a **separate** `--logic-worker`
  process (`worker after click: state=running`);
- press → move off the button → release does **not** click (drag cancel);
- a second click toggles the worker back to `idle`;
- the second button routes its own declared `submit` event with the field's text as payload and
  moves the worker to `submitted`; a second submit returns to `idle`;
- the session exits 0 with `smoke 2 click(s)` (only the primary button is counted).

With `--keys`:

- `abc` typed into the field produces `Field/change` events and `state=editing`;
- `smoke field final: "abc"` proves the text reached the real control.

With `--ime`:

- the script first makes the precondition explicit by running `fcitx5-remote -o` and
  `fcitx5-remote -s pinyin`, and restores the previous state on exit;
- a real composition produces `smoke IME preedit: n` … `ni hao`, then
  `smoke IME commit: 你好` and `smoke field final: "你好"`.

What this is, precisely:

- the event path is now the designer's model, not a direct GTK → channel shortcut. `click`,
  `change` and `submit` are declared in `Registry`; each control is wired in `Wiring`; each wiring
  is bound to an action in `Dispatcher`. The bridge refuses an undeclared event, an invalid payload,
  an unwired node/event and an unbound action, and logs `event refused` instead of sending it.
- `--keys` works on the pinned revision because the host grants GTK focus (`grab_focus`) and gives
  the toplevel X input focus (`XSetInputFocus`). The upstream gap — pointer-press focus routing and
  `ValueChanged` on the key path — is still open; see
  [`docs/rw-feedback/feedback-20261015.md` §17](../../docs/rw-feedback/feedback-20261015.md).
- `--ime` is not a claim that every environment composes. If fcitx is inactive or on its
  `keyboard-us` method, GTK correctly receives one commit per letter; the script fails rather than
  reporting that as a pass.
- the design-time panels for editing declarations and bindings (`panel.events` / `panel.data`) and
  arbitrary control trees are not part of this smoke. The smoke document exists to exercise the
  runtime model with real controls.
