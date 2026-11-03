# 工程格式 v3 提案：`collections` 试点（2026-11-03）

> 状态：**已实现（页面格式 v3 + 只读 `panel.items`）**。证据见
> [log-20261103-1.md](../log/log-20261103-1.md) §36。
>
> 依据：[principle.md](./principle.md) #6/#9/#22/#26/#35；[decisions.md](./decisions.md) Q6/F1；
> [roadmap.md](./roadmap.md) T27.01、§2.2 `panel.items`；前一步
> [`format-v2-proposal.md`](./format-v2-proposal.md)（`states`）。

## 1. 为什么是 v3

`panel.items` 要求“表列/树项/标签页、角色、数据源、选择和编辑规则”。宿主的
[`collection::content`](../../src/app/collection/content.rs) 已经实现列/行/角色/来源，但页面文档
v2 只有 `states`，没有集合内容的位置。v3 增加 `collections`，先把 **表/列表内容** 持久化；
树项、容器页与单元格编辑规则是另外的模型，尚未进入页面成员。

## 2. 字段形状

页面文档版本 3 在 v2 的成员之后增加 `collections`（数组，没有内容时写 `[]`）：

```json
{
  "format": 3,
  "page": { "id": 1, "name": "Main", "root": 1 },
  "nodes": [
    { "id": 1, "type": "window", "children": [2] },
    { "id": 2, "type": "table" }
  ],
  "states": null,
  "collections": [
    {
      "node": 2,
      "origin": "sample",
      "columns": [
        { "id": 1, "header": "Name", "role": "text" },
        { "id": 2, "header": "Done", "role": "check" },
        { "id": 3, "header": "Next", "role": "reference:1" }
      ],
      "rows": [
        {
          "id": 1,
          "cells": [
            { "column": 1, "value": "Write" },
            { "column": 2, "value": "false" }
          ]
        }
      ],
      "source": null
    }
  ]
}
```

| 成员 | 类型 | 含义 |
|---|---|---|
| `collections[].node` | 整数 | 该内容所属的控件节点；必须是页面声明的节点 |
| `collections[].origin` | `sample` / `source` | 样例内容 / 数据源内容 |
| `collections[].columns[].role` | 字符串 | `text`、`check`、`badge:<label>`、`reference:<row>` |
| `collections[].rows[].cells[]` | 对象 | `column` 是列身份，`value` 是写入文本 |
| `collections[].source` | 对象或 null | `adapter`、`provides`、`availability`（`online`/`offline`） |

角色用一条字符串而不是嵌套对象，是为了让“角色”在文件中仍是一个标量，且 `badge` 的标签可以
包含冒号（只按第一个冒号切分）。

## 3. 校验规则（读取时拒绝并定位）

1. `node` 必须是页面 `nodes` 中声明的节点；同一节点只能有一份内容。
2. `origin` 只能是 `sample` 或 `source`。
3. 列 id 唯一、header 非空；`badge:` 的标签非空；`reference:` 指向本集合中存在的行。
4. 行 id 唯一；cell 的 `column` 必须是已声明列；同一行同一列最多一个 cell。
5. `origin = source` 必须有 `source`；`origin = sample` 不能带 `source`。
6. `source.adapter` 非空；`provides` 的 header 非空且不重复；`availability` 只能是两值之一。
7. 未知成员在 collection / column / row / cell / source 内保留并报告（F1）。

## 4. 迁移与写出

- `PAGE_FORMAT` 2 → **3**；读取 v1/v2 时 `collections` 为空，`migrated_from` 记录读到的版本。
- 写出顺序固定：`format`、`page`、`nodes`、`states`、`collections`。
- 保存继续走 `PageDocument::save()`（`Saving::plan_guarded`）；v1/v2→v3 时保留
  `<file>.v<from>.bak`。

## 5. 已实现

| 层 | 文件 | 变更 |
|---|---|---|
| 格式 | `src/project/record/page.rs` | `PageCollection`/`PageColumn`/`PageRow`/`PageCell`/`PageSource`/`PageOrigin`/`PageCellRole`/`PageAvailability`；`decode_collections`/`encode_collection`；7 条校验 |
| 模型 | `src/app/collection/content.rs` | `Collection::restore(...)`：从文档重建，延续最大 id，校验角色/来源规则 |
| GUI | `src/app/run/workbench_items.rs`（新） | `rows(source, collections)`：复用 `Collection::render()`；树项/容器页/编辑规则显式标注未声明 |
| GUI | `src/app/run/workbench_gui.rs` | `panel.items` 注册、与 `states`/`data` 同槽、显示时读取 startup 页面 |
| i18n | `vocabulary.rs`、`language/*.json` | `ui.panel.items` 与 4 个空态键 |

## 6. 尚未实现

1. 树项（`collection::tree`）与容器页（`collection::pages`）的页面成员；
2. 单元格编辑规则（`collection::delegate`）与选择状态的持久化；
3. `panel.items` 的编辑与保存事务；
4. GUI 页面节点接入画布（T02/T03/T09），使画布与集合内容同源。
