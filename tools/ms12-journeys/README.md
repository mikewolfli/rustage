# MS12 journey harness

`run.py` reproduces the joint-acceptance journeys of milestone MS12 (blue13). It is
an integration gate, not a feature: every step records whether it used real files
or processes, only the typed model, or is blocked by a missing capability.

```sh
cargo build --bin rustage
python3 tools/ms12-journeys/run.py
```

The JSON report is written to `target/ms12-journeys/report.json` by default; that
path is a build product and is not committed. Use `--report` to put it elsewhere,
`--binary` to test another build, and `--skip real` for a fast model-only pass
(slow builds are then recorded as blocked, not silently skipped).

A non-zero exit means at least one step failed. Blocked steps do not fail the run
by themselves, but they are counted separately: the point of the gate is that a
reader can tell "this ran and passed" apart from "this machine cannot do it yet".
The evidence boundaries are written up in `docs/log/log-20261020-3.md`.
