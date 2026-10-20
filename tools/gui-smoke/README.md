# GUI smoke (`tools/gui-smoke`)

Real-window evidence for the Linux GTK host. It is not part of `cargo test`: it needs an X11
session, GTK3, `xwininfo`, `xwd`, and `libXtst`.

```text
cargo build --features linux-native --bin rustage
python3 tools/gui-smoke/run.py
python3 tools/gui-smoke/run.py --keys
python3 tools/gui-smoke/run.py --ime    # needs a running fcitx5 with pinyin
python3 tools/gui-smoke/palette.py     # workbench command palette; starts its own Xephyr
python3 tools/gui-smoke/property.py    # workbench Inspector property editing
python3 tools/gui-smoke/scene.py       # workbench object tree + design canvas
python3 tools/gui-smoke/debug.py       # workbench debug panel over a real gdb DAP session
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

- the script clicks the field first, so the library's own pointer-press focus path is under test
  rather than a host-side `focus_widget` workaround;
- `abc` typed into the field produces `Field/change` events and `state=editing`;
- `smoke field final: "abc"` proves the text reached the real control.

With `--ime`:

- the script first makes the precondition explicit by running `fcitx5-remote -o` and
  `fcitx5-remote -s pinyin`, and restores the previous state on exit;
- the host registers the library's pre-edit observer (`WidgetHandle::on_ime_preedit`), so each
  uncommitted frame is logged (`smoke IME preedit: "ni hao"@0`) before the commit;
- a real composition commits `你好` (`smoke field final: "你好"`). The final Chinese string is the
  observable proof that composition happened rather than five separate letter commits.

With `palette.py`:

- the script starts a nested X server (Xephyr) by default, so a locked or busy developer desktop
  is not mistaken for a working input target; set `PALETTE_DISPLAY=:N` to use an existing display;
- it clicks the workbench's first toolbar button, types a query into the real line edit, clicks
  **Run**, and checks the host's own log (`palette opened`, `palette query "welcome": 1 result(s)`,
  `palette ran view.welcome`, `palette closed; focus Some(...)`);
- it checks that focus returns to the control that had it before the palette opened, that **Close**
  hides the palette, and that clicking a result row selects the command Run then executes;
- **Enter/Escape are reported as blocked, not passed**: on this pinned revision the GTK key path
  forwards raw GDK keyvals (`Return` = `0xFF0D`, `Escape` = `0xFF1B`) while the library's
  `LineEdit` matches `13`/`27`, so `return_pressed`/`editing_finished` do not fire. The defect is
  recorded in `docs/rw-feedback/feedback-20261020.md`; the palette closes through its **Close**
  control until upstream normalizes the keyval.

With `scene.py`:

- the Outline list is the tree view of the same scene the canvas paints: `window`, `Demo`,
  `button`, `label`, `hidden note [hidden]`, `footer [locked]`;
- a hidden row is refused with `node 4 is hidden` and a locked row with `node 5 is locked`; the
  Outline snaps back to the node the scene actually selected instead of showing a refused row;
- clicking the overlap selects the topmost node (`label`), clicking outside it selects the `button`,
  and a tree row selects the same node through the other view;
- a drag in empty canvas selects every selectable node it crosses (`marquee ...: 4 node(s):
  0 window, 1 Demo, 2 button, 3 label`) and the hidden note is skipped;
- the Inspector follows the selection, so the row reads `title: label` after the canvas click.

With `debug.py`:

- the panel's **Start** button spawns a worker that compiles a demo debuggee with the host's own
  `rustc` (cached when the source and command are unchanged), starts the adapter the probe found,
  and binds a breakpoint;
- the host log reports `debug: stopped: breakpoint at line 3`, `bound at 3`, `thread 1 app`, the
  top frame, `scope Locals`, and `variable n: i32 = 41`; these rows come from the adapter's own
  `threads`/`stackTrace`/`scopes`/`variables` answers, not from the model;
- **Step** stops on the next line, **Run** (continue) runs the program to its exit, and **Stop**
  ends the second session; the worker is not started unless the button is pressed;
- two-way source navigation is **not** claimed: the panel shows the raw `main.rs:3:0` position and
  says `unmapped: no generated source map loaded` rather than guessing a node;
- the probe starts its own Xephyr and registers a C-level `atexit` cleanup, so an Xlib fatal exit
  cannot leave the window or the nested server behind.

With `property.py`:

- it clicks the Inspector value field, types an over-limit title and clicks **Apply**; the host
  reports `revision 0 unchanged` and the Inspector row still reads `title: Demo`;
- **Clear** empties the field without changing the valid value; typing `test` and applying gives
  `inspector applied title = "test"; revision 1` and `inspector rows: title: test`;
- the double-click/Enter entry is **not** claimed: the pinned GTK backend does not synthesise
  `MouseDoubleClick` and its keyval path is the one in §0.2. The visible field is the real entry.

What this is, precisely:

- the event path is now the designer's model, not a direct GTK → channel shortcut. `click`,
  `change` and `submit` are declared in `Registry`; each control is wired in `Wiring`; each wiring
  is bound to an action in `Dispatcher`. The bridge refuses an undeclared event, an invalid payload,
  an unwired node/event and an unbound action, and logs `event refused` instead of sending it.
- `--keys` works on the pinned revision because the library moves router focus on a pointer
  press (`focus_on_pointer_press`) and its GTK window grabs keyboard focus on `map`; the host no
  longer installs a focus workaround. `ValueChanged` is injected by the library too, so the
  host's de-duplicated watcher is only a second observer for IME commits.
- `--ime` is not a claim that every environment composes. If fcitx is inactive or on its
  `keyboard-us` method, GTK correctly receives one commit per letter; the script fails rather than
  reporting that as a pass.
- the design-time panels for editing declarations and bindings (`panel.events` / `panel.data`) and
  arbitrary control trees are not part of this smoke. The smoke document exists to exercise the
  runtime model with real controls.
