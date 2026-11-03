# 工程格式 v5 提案：`motion` 试点（2026-11-03）

> 状态：**已实现（页面格式 v5 + 只读 `panel.timeline`）**。证据见
> [log-20261103-1.md](../log/log-20261103-1.md) §38。
>
> 依据：[principle.md](./principle.md) #6/#9/#22/#26；[decisions.md](./decisions.md) Q6/F1；
> [roadmap.md](./roadmap.md) T11.06、§2.2 `panel.timeline`；前序
> [`format-v2-proposal.md`](./format-v2-proposal.md)、[`format-v3-proposal.md`](./format-v3-proposal.md)、
> [`format-v4-proposal.md`](./format-v4-proposal.md)。

## 1. 字段形状

页面文档版本 5 在 v4 之后增加 `motion`：

```json
{
  "format": 5,
  "page": { "id": 1, "name": "Main", "root": 1 },
  "nodes": [ { "id": 1, "type": "page" } ],
  "states": null,
  "collections": [],
  "components": { "definitions": [], "instances": [], "variants": [] },
  "motion": {
    "motions": [
      { "effect": "transition", "property": "color", "duration_ms": 200, "delay_ms": 0, "easing": "ease-out" },
      { "effect": "animation", "property": "opacity", "duration_ms": 400, "delay_ms": 100, "easing": "linear" }
    ]
  }
}
```

| 成员 | 含义 |
|---|---|
| `effect` | `transition` 或 `animation` |
| `property` | 被移动的属性名，非空 |
| `duration_ms` / `delay_ms` | 0..=4294967295 |
| `easing` | `linear`、`ease-in`、`ease-out`、`ease-in-out` |

## 2. 校验与迁移

1. `property` 非空；effect/easing 必须是枚举内的值。
2. 未知成员在 `motion` 与每条 motion 内保留并报告。
3. `PAGE_FORMAT` 4 → **5**；v1–v4 读取时 `motion` 为空，`migrated_from` 记录版本。
4. 写出顺序固定：`format`、`page`、`nodes`、`states`、`collections`、`components`、`motion`。
5. 保存沿用 `PageDocument::save()`；迁移保留 `<file>.v<from>.bak`。

## 3. 已实现

| 层 | 文件 | 变更 |
|---|---|---|
| 格式 | `src/project/record/page.rs` | `PageMotions`/`PageMotion`/`PageEffect`/`PageEasing`；encode/decode；时间参数 |
| GUI | `src/app/run/workbench_timeline.rs`（新） | 转成 `style::motion::Timeline`/`Control`，复用 `Timing::render()` 与总时长 |
| GUI | `src/app/run/workbench_gui.rs` | `panel.timeline` 注册、与 `states`/`items`/`components` 同槽、读取 startup 页面 |
| i18n | `vocabulary.rs`、`language/*.json` | `ui.panel.timeline` 与 4 个空态键 |

面板行：motion 数量与总时长、每条 `effect property duration (+delay, easing)`、
`playback: paused at 0ms / <total>ms`、`target capability: not checked`、
`state comparison: not declared in page format v5`。

## 4. 尚未实现

1. 实时播放/暂停/擦洗（`Control` 已就绪，但 GUI 没有帧泵）；
2. 目标能力表（`style::motion::Capability` 的来源未接入目标契约）；
3. 状态比较（需要 T11.05 的交互状态来源）；
4. `panel.timeline` 的编辑与保存；
5. GUI 页面节点接入画布（T02/T03/T09）。
