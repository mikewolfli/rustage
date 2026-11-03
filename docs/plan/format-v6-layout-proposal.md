# 工程格式 v6 提案：`layout` 约束（2026-11-03）

> 状态：**v6 最小切片已实现（2026-11-03，log §48）**。落地的只有“容器声明”：
> 页面根节点的 `layout` 对象（`{ "type": …, 参数… }`）原样进入库文档，stripped 产物带常量坐标，
> 预览与生成共用同一份 JSON。每节点尺寸约束等字段仍缺（upstream RW-109/RW-111），因此本文件
> §2 的 `nodes[]` 约束表仍是**未实现**部分。
>
> **2026-11-03（4.0.0 `1a30785`）复核：** 上游生成器已消费**根节点**的 `layout` 声明
> （`JsonProject::layout_declaration()` → `plan_geometry`）并把 `json::layout_schema` 的诊断放进
> `GenerationReport.layout_issues`；RW-47 的“完全不看 layout”结论作废，RW-42 的逐节点锚点也以
> `GenerationReport.node_anchors` 侧信道提供。仍未解决的是**嵌套容器**与**每节点约束**：上游只解根
> 容器一层。详见 [feedback-20261103.md](../rw-feedback/feedback-20261103.md) §RW-108。
>
> 依据：[roadmap.md](./roadmap.md) T10.01–T10.04、§2.2 `panel.layout`；
> [format-v2-proposal.md](./format-v2-proposal.md) … [format-v5-proposal.md](./format-v5-proposal.md)
> 的版本/迁移约定；[decisions.md](./decisions.md) Q6/F1。

## 1. 为什么不能先画面板

`panel.layout` 要求“容器、约束、尺寸策略、边距、间距、布局诊断”。这些不是展示字段：

- **预览**必须用 `rust_widgets::layout` 的 `Hints` / `Layout` / `LayoutContext` 计算同一棵树，
  不能由面板自己算一套（#14）。
- **生成**必须把同一组约束映射为 `FlexLayout` / `GridLayout` / `Absolute` 的构造调用；如果
  面板显示的约束不进入生成，用户会得到“设计器里对了、生成后不对”的假成功（#13/#22）。
- **CSS 只负责外观**，布局结构不能塞进 CSS（F1）。因此约束必须进 JSON 文档。

在 T10.01/T10.03 的 `Layout::update/arrange_with_context` 与 `Hints` 规范化接入前，只画面板会
是第二条事实来源（#59）。所以本轮只写提案。

## 2. 推荐字段形状（v6）

页面文档 v6 增加 **节点级的 `layout` 对象**。它和 `nodes[].properties` 分开：properties 是
控件自己的标量属性，`layout` 是父容器如何安置这个节点。

```json
{
  "format": 6,
  "page": { "id": 1, "name": "Main", "root": 1 },
  "nodes": [ { "id": 1, "type": "window" }, { "id": 2, "type": "panel" } ],
  "layout": {
    "container": {
      "node": 1,
      "kind": "flex",
      "direction": "row",
      "spacing": 8,
      "align": "start",
      "justify": "start",
      "padding": [8, 8, 8, 8]
    },
    "nodes": [
      {
        "node": 2,
        "width": { "kind": "fill" },
        "height": { "kind": "fixed", "value": 48 },
        "min": [0, 0],
        "max": [null, null],
        "margin": [0, 8, 0, 8],
        "overflow": "clip"
      }
    ]
  }
}
```

| 成员 | 值 | 说明 |
|---|---|---|
| `container.kind` | `flex` / `grid` / `absolute` | 只列库真正提供的容器语义 |
| `container.direction` | `row` / `column` | flex 用 |
| `container.spacing` | 非负整数 | 子节点间距 |
| `container.align` / `justify` | `start` / `center` / `end` / `stretch` | 与库枚举一致 |
| `container.padding` | 四元整数数组 | 容器内边距 |
| `nodes[].width` / `height` | `{ "kind": "fixed"/"fit"/"fill", "value": n }` | 尺寸策略 |
| `nodes[].min` / `max` | 二元数组，`null` 表示不限 | 最小/最大尺寸 |
| `nodes[].margin` | 四元整数数组 | 外边距 |
| `nodes[].overflow` | `clip` / `visible` / `scroll` | 溢出策略 |

**不放进 v6**：绝对坐标（`absolute` 容器仍由布局引擎计算，不写隐式 x/y）、CSS 过渡
（`motion` 已负责）、断点/自适应视口（H-29 需要另一字段，见 §6）。

## 3. 与上游模型的对应

| 文档字段 | 上游 |
|---|---|
| `width/height { kind: fixed, value }` | `Hints.min/pref/max` 的规范化值 |
| `{ kind: fit }` | `Hints.pref = Auto`（内容决定） |
| `{ kind: fill }` | 主轴 `fill` / 交叉轴 `stretch` |
| `margin` / `padding` | `Inset` / `Padding` |
| `spacing` | `FlexLayout` 的 `gap` |
| `overflow` | `Overflow` |
| `align` / `justify` | `Align` / `Justify` |
| `container.kind` | `FlexLayout` / `GridLayout` / `AbsoluteLayout` |

映射必须在 **T10.03 的规范化层**完成，文档不直接写引擎结构体，避免字段与上游实现绑死
（#36/#37）。

## 4. 校验规则

1. `container.node` / `nodes[].node` 必须是页面声明的节点；每个节点最多一条 layout 记录。
2. `container.kind` 只接受三种；`direction` 只接受两种；`align/justify` 只接受四值。
3. `spacing`、`padding`、`margin`、`min/max` 必须非负；`fixed.value` 必须为正。
4. `min <= max`（两端都存在时）；`fixed` 必须落在 `[min, max]` 内。
5. `absolute` 容器下的子节点不接受 `fill`（它会退化为隐式坐标，违反 #14）。
6. 未知成员保留并报告（F1）。
7. 诊断由 `layout::Diagnostic` 产生：非法/冲突约束定位到具体节点与来源，面板只呈现。

## 5. 迁移与生成

- `PAGE_FORMAT` 5 → **6**；v1–v5 读取时 `layout` 为空，`migrated_from` 记录版本。
- 写出顺序固定：`format`、`page`、`nodes`、`states`、`collections`、`components`、`motion`、
  `layout`。
- 保存沿用 `PageDocument::save()`；迁移保留 `<file>.v<from>.bak`。
- **生成**：T15.02 的 `designer` 管线必须消费同一 `layout` 输入；不能消费时生成报告必须阻断
  或明确“layout 未参与生成”，不得静默丢弃（#13/#22）。

## 6. 仍需确认

1. **容器归属**：`layout.container` 放在页面级还是每个容器节点上？本提案是页面级单容器示例；
   嵌套容器需要按节点存放。
2. **尺寸策略名称**：`fixed`/`fit`/`fill` 是否与库的 `Hints` 术语一致，还是直接用 `min/pref/max`？
3. **绝对布局**：是否允许 `absolute` 作为首版容器？原则 #14 要求不悄悄改成绝对坐标，但显式
   `absolute` 是库支持的语义。
4. **断点/自适应**：H-29 需要文档模型增加断点字段；是否与 v6 同批，还是 v7？
5. **CSS 交互**：`width`/`height` 在 CSS 与 layout 中同时出现时，哪一个是权威？本提案取
   JSON layout 为权威，CSS 只做外观，但需要在 T11.07 的检查器中显式诊断冲突。

## 7. 实施前置

1. T10.01/T10.03 的 `Layout::update/arrange_with_context`、`Hints` 规范化与 `ControlMetrics`
   接入；
2. T10.02 的容器切换与子节点约束迁移；
3. T15.02 的生成器消费与 gap 诊断；
4. T11.07 的 CSS/layout 冲突诊断。

四项中任一未完成，`panel.layout` 保持 `Missing`，不画空面板（#26）。

## 8. 4.0.0 复核后的推荐答案（待确认，未实施）

按原则 #6/#14/#59，对 §6 的五个问题给出推荐，供确认后一次性落地：

| 问题 | 推荐 | 理由 |
|---|---|---|
| 1. 容器归属 | **页面级单容器先落地，形状与上游一致**：`layout` 放在根节点上（`nodes[]` 里声明为根的那一个），不是文档顶层；嵌套容器等上游支持每节点声明后再加 | 上游 `plan_geometry` 只读 `JsonProject::layout_declaration()`（根节点），宿主另造页面级字段会立刻成为第二条路径（#59） |
| 2. 尺寸策略名称 | 文档继续写 `fixed`/`fit`/`fill`，在 T10.03 映射到 `Hints.min/pref/max` | 文档不直接写引擎结构体（#36/#37）；`fill` 在主/交叉轴的不同含义只在映射层解释一次 |
| 3. 绝对布局 | **允许显式 `absolute`**，但只作为容器 `kind`，不引入任何隐式 x/y | 库有 `AbsoluteLayout`；禁止的是“悄悄退化为绝对坐标”，不是显式语义（#14） |
| 4. 断点/自适应 | **v7**，不与 v6 同批 | H-29 需要“一份文档三档视口”的预览路径，而宿主现在只有一个 `--project` 会话；把两个未验证的概念塞进一次格式升级无法区分失败原因 |
| 5. CSS 与 layout 的权威 | **JSON layout 权威，CSS 只做外观**；冲突由 T11.07 诊断，不静默取一个 | 与 F1“布局不进 CSS”一致；诊断让冲突可见（#22） |

**2026-11-03 第二轮核对：前置不是“没做”，而是“做完了也进不了生成”。**

| 前置 | 实况 |
|---|---|
| T10.01/T10.02/T10.03 | ✅ 已实现（blue21 覆盖率 F10 8/8）：`layout::layout_plan`、`constraints::BoxConstraints::validate`、`layout_choice::switch/detach`（不能表达的转换拒绝）。 |
| T11.07 | ✅ 已实现（`style::style_emit`：预览与生成同输入一致）。 |
| 生成器消费 | ⚠️ 只消费**根节点** `layout` 与 child 的 `stretch/col/row/col_span/row_span/region/extent`（`json/layout_schema.rs:266-274`、`designer/generator.rs:2223`）；**没有** min/max/pref、每节点 margin、overflow 的字段 → 新登记 [RW-109](../rw-feedback/feedback-20261103.md)。 |
| 预览同语义 | ⚠️ 宿主预览挂载的是自己的 `preview::Document`（`preview/preview_service.rs`），不是生成器消费的库 JSON 文档；但 4.0.0 已提供 `json::JsonViewSession`（JSON ↔ ViewEngine，保留控件身份，见 [feedback §RW-110](../rw-feedback/feedback-20261103.md)），所以这条是**宿主未迁移**，不是上游缺口。 |

因此 `panel.layout` 仍保持 `Missing`，但原因已精确为两条：

1. **RW-109**：尺寸约束在上游文档里没有拼写；
2. **预览与生成不是同一份文档**（T10.01 的“复用同语义”只在宿主布局缝内成立，未覆盖生成）。

**推荐先做第 2 条**：把预览改为用 `JsonViewSession` 挂载 `to_library_document` 的产物。它不依赖
上游修复，且直接把布局语义收敛到一份文档；代价是要同时搬动 T14.02 的活体编辑与 T14.03 的状态
保持断言。RW-109 可以在这条路径落地后再补，届时只需在 JSON 里增加字段，不需要再改一次架构。
在此之前不实现 `layout` 成员，避免第三条布局路径。
