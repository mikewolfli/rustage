# 页面格式 v7 提案与实现记录：每节点 `constraints`

日期：2026-11-03
状态：**已实现**（`src/project/record/page.rs`、`src/app/generate/document.rs`、
`src/app/run/workbench_layout_panel.rs`）。
依据：[principle.md](principle.md) #6/#9/#13/#14/#26/#27；上游
[rust-widgets 4.0.1](https://github.com/mikewolfli/rust-widgets) 的 RW-109 实现
（`json::ChildLayoutAttrs`、`src/designer/generator.rs` 的 `child_hints`/`child_margin`）；
[blue22.md](blue22.md) RW-109 行；证据 [log-20261103-2.md](../log/log-20261103-2.md)。

## 1. 为什么需要 v7

页面格式 v6 的根节点可以声明容器 `layout`，但没有任何地方能表达“这个子节点至少 120 宽”
“这个子节点四周留 8”或“溢出裁剪”。4.0.1 起库的声明式文档在**节点属性对象**上读取这些
键（`min_width`/`min_height`/`pref_width`/`pref_height`/`max_width`/`max_height`/`margin`/
`overflow`），并把它们送入 generation-time solve；生成器还会通过
`GenerationReport::layout_issues` 诊断“本布局不支持该字段”。

宿主因此需要一个**稳定的文档位置**来保存这些声明，而不是把它们混进控件属性表（那会让
`min_width` 看起来像某个控件的属性）或用宿主自己的约束语言（那是原则 #14 禁止的第二套布局）。

## 2. 形状

```json
{
  "id": 2,
  "type": "button",
  "children": [],
  "properties": [ { "name": "text", "value": { "text": "Go" } } ],
  "layout": null,
  "constraints": {
    "min_width": 120,
    "pref_height": 32,
    "margin": [8, 4],
    "overflow": "clip"
  }
}
```

- `constraints` 是节点成员，缺省写 `null`（与其他成员“一个页面一种拼写”一致）。
- 成员顺序按文档书写顺序保留；未知成员保留并交由库诊断，不在宿主复制属性表。
- `margin` 接受整数或 1/2/4 个整数（与库的 `insets_from_value` 同规则）；其他长度是打字错误，
  读取时拒绝。
- `overflow` 只接受 `clip`/`visible`/`scroll`；4.0.1 没有运行期载体，生成报告会把它列为
  dropped field，宿主面板照实显示。
- **只允许非根节点**：根节点由生成请求的宽高放置，写在其上的约束会被本构建读取而被 solve
  忽略，因此读取时拒绝并给出 `nodes[<id>].constraints` 路径（原则 #13 的“不得静默丢弃”）。

## 3. 生成翻译

`to_library_document` 把每个约束成员写成该节点属性对象上的**扁平键**，例如：

```json
{ "button": { "id": "2", "text": "Go", "min_width": 120, "margin": [8, 4] } }
```

这正是 `ChildLayoutAttrs::from_value` 读取的位置，也是生成期 `child_hints` 的输入；嵌套一个
`"constraints"` 对象会让库看不懂，是多此一举的第二套拼写。

测试 `a_declared_minimum_height_reaches_the_generated_solve` 用 vbox + `min_height: 80` 读取
`GenerationReport::node_rects`，断言子节点高度 ≥ 80，证明声明真的到达了 solve，而不只是被
文件保存。

## 4. 兼容与迁移

- `PAGE_FORMAT` 6 → 7；`PAGE_OLDEST_READABLE` 仍为 1。
- v1–v6 文档读取时 `constraints` 为空，`migrated_from` 记录读到的版本；写回为 v7。
- v6 文档里若出现 `constraints` 成员，不按 v7 解释（留在 `preserved`），避免用旧版本的规则
  读新版本的字段。
- 写文件时即使缺省也写 `"constraints": null`，因此“同一页面同一字节串”仍然成立。

## 5. 未包含

- 断点/分档子树：JSON 声明式文档没有该字段，宿主不发明（RW-117）。
- 预览（模式 1）的布局重解：库的 `json_to_view_tree` 不消费根 `layout`（RW-117/118），因此
  `constraints` 目前只影响生成；`--preview tiers:<page>` 用生成期 solve 展示三档结果。
