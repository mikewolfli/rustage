# 工程格式 v4 提案：`components` 试点（2026-11-03）

> 状态：**已实现（页面格式 v4 + 只读 `panel.components`）**。证据见
> [log-20261103-1.md](../log/log-20261103-1.md) §37。
>
> 依据：[principle.md](./principle.md) #6/#9/#22/#26/#35；[decisions.md](./decisions.md) Q6/F1；
> [roadmap.md](./roadmap.md) T12.01–T12.03、§2.2 `panel.components`；前序
> [`format-v2-proposal.md`](./format-v2-proposal.md)（`states`）、
> [`format-v3-proposal.md`](./format-v3-proposal.md)（`collections`）。

## 1. 为什么是 v4

`panel.components` 要求“组件定义、实例、变体、覆盖、版本、目标兼容”。宿主的
[`component::definition`](../../src/app/component/definition.rs) 与
[`component::variant`](../../src/app/component/variant.rs) 已经实现这些模型，但页面文档 v3
没有存放它们的位置。v4 增加 `components`，把定义/实例/变体写进页面文档。

## 2. 字段形状

页面文档版本 4 在 v3 的成员之后增加 `components`（对象；没有内容时写空数组）：

```json
{
  "format": 4,
  "page": { "id": 1, "name": "Main", "root": 1 },
  "nodes": [
    { "id": 1, "type": "page", "children": [2] },
    { "id": 2, "type": "panel" }
  ],
  "states": null,
  "collections": [],
  "components": {
    "definitions": [
      {
        "name": "Card",
        "version": 2,
        "parameters": [ { "name": "title", "default": { "text": "Hi" } } ],
        "slots": ["body"],
        "body": [
          { "control": "panel", "properties": [ { "name": "title", "value": { "text": "Card" } } ] },
          { "nested": "Badge" },
          { "slot": "body" }
        ]
      },
      { "name": "Badge", "version": 1, "parameters": [], "slots": [], "body": [ { "control": "label", "properties": [] } ] }
    ],
    "instances": [
      {
        "definition": "Card",
        "node": 2,
        "arguments": [ { "name": "title", "value": { "text": "Save" } } ],
        "fills": [ { "slot": "body", "parts": [ { "control": "label", "properties": [] } ] } ]
      }
    ],
    "variants": [
      {
        "instance": 2,
        "definition": "Card",
        "kind": "panel",
        "targets": ["desktop"],
        "overrides": [ { "name": "title", "value": { "text": "Save" } } ]
      }
    ]
  }
}
```

| 成员 | 含义 |
|---|---|
| `definitions[].parameters[]` | `name` + 可选 `default`（值族与节点属性相同） |
| `definitions[].slots[]` | 槽名 |
| `definitions[].body[]` | `control`（kind + properties）、`nested`（定义名）或 `slot`（槽名），恰好一个 |
| `instances[].node` | 实例的文档身份；必须是页面节点 |
| `instances[].fills[]` | `slot` + `parts`（与 body 相同的 part 形状） |
| `variants[].targets` | 五个 profile 名之一，或未知目标（面板逐条报告） |

## 3. 校验规则

1. 定义名非空且唯一；参数名、槽名各自唯一且非空；body 引用的槽必须声明。
2. `nested` 引用的定义必须存在；定义图不得成环（报 `a -> b -> a`）。
3. 实例的定义必须存在；`node` 必须是页面节点且只能是一个实例；参数名必须是定义声明的；
   fill 的槽必须声明。
4. 变体必须指向一个实例；`definition` 必须与实例一致；kind 非空；target 非空且不重复；
   override 名非空。
5. 值族复用节点属性的 `encode_property_value` / `decode_property_value`；非有限数字拒绝写出。
6. 未知成员在 components / definition / parameter / fill / instance / variant 内保留并报告。

## 4. 迁移与写出

- `PAGE_FORMAT` 3 → **4**；v1/v2/v3 读取时 `components` 为空，`migrated_from` 记录版本。
- 写出顺序固定：`format`、`page`、`nodes`、`states`、`collections`、`components`。
- 保存继续走 `PageDocument::save()`；迁移时保留 `<file>.v<from>.bak`。

## 5. 已实现

| 层 | 文件 | 变更 |
|---|---|---|
| 格式 | `src/project/record/page.rs` | `PageComponents`/`PageDefinition`/`PageParameter`/`PagePart`/`PageInstance`/`PageFill`/`PageVariant`；encode/decode；图校验 |
| GUI | `src/app/run/workbench_components.rs`（新） | 转成 `Registry`/`Variant`，复用 `insert`/`place`/`target_compatibility` |
| GUI | `src/app/run/workbench_gui.rs` | `panel.components` 注册、与 `states`/`items` 同槽、读取 startup 页面 |
| i18n | `vocabulary.rs`、`language/*.json` | `ui.panel.components` 与 4 个空态键 |

## 6. 尚未实现

1. `panel.components` 的编辑（定义/实例/变体）与保存事务；
2. 目标兼容与工程当前 target 的对照（面板只检查目标名是否合法，并说明不读工程 target）；
3. GUI 页面节点接入画布（T02/T03/T09）；
4. 生成器对 `components` 的展开诊断（**已实现为生成拒绝**：`unconsumed_by_generator()` 把 components 列入未消费成员）。
