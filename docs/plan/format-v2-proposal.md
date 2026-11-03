# 工程格式 v2 提案：`states` 试点（2026-11-03）

> 状态：**已实现（格式 v2 + 只读 `panel.states`，2026-11-03）**。字段、校验、迁移与接线顺序已落地；
> 编辑/保存与生成器消费仍按 §5 的后续步骤。实现证据见
> [log-20261103-1.md](../log/log-20261103-1.md) §35。
>
> **后续：** 页面格式已在 v3 增加 `collections`（见 [`format-v3-proposal.md`](./format-v3-proposal.md)）；
> 当前 `PAGE_FORMAT` 是 3，本文的 v2 字段仍是 v3 文档的一部分。
>
> 依据：[principle.md](./principle.md) #6/#9/#14/#22/#26/#54；[decisions.md](./decisions.md)
> Q6（版本化迁移）、U1（向前迁移）、F1（单一工程根、未知字段保留、字段增删必须走版本）；
> [roadmap.md](./roadmap.md) T13.06（显式状态机属宿主）、§2.2 `panel.states`；
> [blue21.md](./blue21.md) §2.2 开放项；[log-20261103-1.md](../log/log-20261103-1.md) §33。

## 1. 为什么需要 v2

`panel.states` 要求“状态、守卫、动作、过渡、目标能力与运行状态”，宿主已有
[`data::state_machine`](../../src/app/data/state_machine.rs) 的 `Machine` / `Session` / `Transition`，
但没有把它放进任何持久化文档：

- 页面文档 v1 的成员只有 `format` / `page` / `nodes`，节点的 `properties` 只接受标量
  （`PropertyValue` 的七种值族），无法表达状态机。
- 若只在 GUI 内存里放一个 `Machine`，重启后状态机丢失，且它与页面文档会成为两个事实来源
  （原则 #6）；若另写一个未版本化文件，则违反 F1/Q6。

因此需要一个 **版本化的页面文档成员**。本提案选择 `states` 作为第一个试点，因为它不依赖
控件树的生成语义，且能直接复用已有的 `Machine` 校验。

## 2. 字段形状（提案）

页面文档版本 2：

```json
{
  "format": 2,
  "page": { "id": 1, "name": "Main", "root": 2 },
  "nodes": [
    { "id": 2, "type": "panel", "children": [3] },
    { "id": 3, "type": "button", "properties": [{ "name": "text", "value": { "text": "Start" } }] }
  ],
  "states": {
    "declared": ["idle", "running", "done"],
    "initial": "idle",
    "final": ["done"],
    "transitions": [
      { "from": "idle", "on": "start", "to": "running" },
      { "from": "running", "on": "finish", "to": "done", "guard": "is_complete" }
    ]
  }
}
```

| 成员 | 类型 | 必需 | 含义 |
|---|---|---|---|
| `states.declared` | 字符串数组 | 是（`states` 存在时） | 状态名，按声明顺序；不得为空、不得重复、不得含空白 |
| `states.initial` | 字符串 | 否 | 初始状态；缺省表示“未指定初始状态”，不是“第一个状态” |
| `states.final` | 字符串数组 | 否 | 终止状态；缺省为空 |
| `states.transitions` | 对象数组 | 否 | 每条 `from` / `on` / `to` 必需，`guard` 可选 |
| `states.transitions[].guard` | 字符串 | 否 | 守卫名；缺省表示无条件 |

**不放进 v2 的**：状态动作（`action`）、目标能力、运行状态。前两者没有宿主模型；运行状态是
会话事实，不写入文档。`panel.states` 对它们显示“未声明 / 未运行”，不伪造。

## 3. 校验规则（读取时全部拒绝并定位）

1. `states` 必须是对象；`declared` 必须是字符串数组且非空。
2. 状态名不能为空、不能含空白、不能重复。
3. `initial`（若存在）必须在 `declared` 中；`final` 的每一项都必须在 `declared` 中。
4. 每条 transition 的 `from` / `to` 必须在 `declared` 中；`on` 不能为空。
5. `guard`（若存在）不能为空字符串。
6. 未知成员保留并报告（F1），不静默丢弃。
7. 同一页面只能有一台机器；多机器/组件级状态机留待后续版本。

这些规则直接映射到 `Machine` 的 `unreachable` / `dead_ends` 诊断：文档非法时拒绝读取，
文档合法但图有问题时在面板显示（不是读取失败）。

## 4. 迁移与写入

- `PAGE_FORMAT` 1 → **2**。读取 v1 时按“没有 `states`”解释，并记录 `migrated_from = v1`。
- 写回 v1 文档时先保留原文件（与项目文档迁移同一规则：迁移不覆盖读到的文件），再写 v2；
  原文件与备份的命名规则沿用 `project/record/format.rs` 的迁移路径。
- 读取 v2 时缺 `states` 等于“没有状态机”（可选成员，语义已定义）；读取 v3 拒绝并说明版本。
- 写出顺序固定：`format`、`page`、`nodes`、`states`；`states` 内部固定
  `declared`、`initial`、`final`、`transitions`，transition 固定 `from`、`on`、`to`、`guard`。
  固定顺序是生成物确定性的前提（原则 #16）。

**为什么不是 v1 的可选成员**：F1/Q6 要求字段增删走版本与迁移；“缺省即没有”只对**同一版本
内**声明为可选的成员成立，不能用来让旧版本悄悄长出新字段。

## 5. 代码影响（已实现部分）

| 层 | 文件 | 变更 |
|---|---|---|
| 格式 | `src/project/record/page.rs` | `PAGE_FORMAT = 2`；`PAGE_DOCUMENT_MEMBERS` 加 `states`；`PageStates` / `PageTransition`；`decode_states` / `encode_states`；v1 读取 + `migrated_from` |
| 格式 | `src/project/record/format.rs` | 未改；页面迁移的备份留给页面保存路径（当前 GUI 不写页面） |
| GUI | `src/app/run/workbench_states.rs`（新） | `rows(source, states)`：`Machine::render()` 的声明/转换/可达性 + 三项未声明的缺口 |
| GUI | `src/app/run/workbench_gui.rs` | `panel.states` 注册、与 `data` 同槽、显示时读取工程首页面文档；只读 |
| CLI | `src/app/run/project.rs` | `--project edit <file>` 打印 `states` 声明，作为格式的真实消费点 |
| 保存 | `src/project/record/page.rs`、`src/app/run/document.rs` | `PageDocument::save()` 走 `Saving::plan_guarded`；v1→v2 写 `<file>.v1.bak`；`--save <file> --save-page` |
| 文档 | `docs/plan/blue21.md`、`docs/log/log-20261103-1.md` | §2.2 `states` 行改为已接线（Hidden）；格式与面板证据见 log §35 |

**尚未实现（按顺序）：**

1. GUI 打开工程时把页面节点接入画布（T02/T03/T09），使画布与 `states` 同源；
2. `panel.states` 的编辑（Add state/Add transition）与保存事务（保存路径已就绪，但 GUI 工程
   当前未被信任，面板仍保持只读）；
3. 生成器对 `states` 的“未消费”诊断（**已实现为生成拒绝**：`PageDocument::unconsumed_by_generator()` + `read_document` 阻断）。

## 6. 四个问题的当前答案（已按推荐默认值落地）

1. **粒度**：页面级（已实现）；组件级留到组件模型落地。
2. **成员名**：`states`（已实现）。
3. **守卫/动作表达**：v2 只存守卫名；动作留给 T13.04 的事件连线（面板行明确显示“not declared”）。
4. **生成语义**：v2 暂不参与生成；生成报告需在后续步骤标注“未消费”（尚未实现）。

以上四项仍可由评审修改；改动字段时按 Q6 增加新的迁移步骤，不直接改 v2 语义。
