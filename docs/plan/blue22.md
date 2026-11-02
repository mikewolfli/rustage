# blue22｜挂起项落地后的另开任务总登记（2026-11-02）

> 依据与承接：[feedback-20261101.md](../rw-feedback/feedback-20261101.md) 四轮扫描 + 第五轮决策记录；
> 评审准则 [principle.md](./principle.md)；任务总账 [roadmap.md](./roadmap.md)；证据矩阵 [support-matrix.md](./support-matrix.md)。
> 基线：宿主固定 rust-widgets **3.2.1（`be69fa2b`）**；probe 契约 **19/19 Verified**（2026-11-02，Linux x86_64，`cargo test probe::`）。
> **本文只做任务登记与排序，不重复扫描证据**：每条的证据行号与消费点见 feedback-20261101.md 对应编号/章节。
> 任务记号：`H-*` 宿主侧改动；`RW-*` 上游请求（blocked upstream）；`VCS-*` 既有缺陷；`M-*` 验证补证（回写支持矩阵）。
> 状态：全部 `open`，另注明依赖。不修改上游代码；不改宿主既有属性/方法/变量名（原则 #34）。
>
> **执行更新（2026-11-03）：** 宿主已升级到 rust-widgets **4.0.0（`1a30785`）**；批次 A 与
> 批次 B 全部关闭，VCS-1 已修复，M-1/M-2 已补证。M-3 被上游 RW-101
> （Windows `WA_INACTIVE` 类型不匹配）阻断；其余未完成项见本文 §7。证据：
> [log-20261103-1.md](../log/log-20261103-1.md)。

## 0. 已关闭（不再计入任务）

| 项 | 结果 | 证据 |
|---|---|---|
| H-9 probe 契约常量过期 | 已修复：`DEPENDENCY_VERSION="3.2.1"`、`DEPENDENCY_REVISION="be69fa2b…"`；drift 测试由 FAILED 转 19/19 全绿 | 第五轮决策 5；`src/probe/contract.rs:39,45` |
| 决策 1 roadmap 基线同步 | roadmap 六处 + decisions/capability-reference 的“当前修订”指针全部改为 3.2.1/`be69fa2b` | 第五轮决策 1 |
| 决策 2 B26 表改写 | roadmap §1.6.2 改为“上游已落地 / 半落地”两段 | 第五轮决策 2 |
| 决策 3 计数口径 | 统一为 230（`WidgetKind`）/268（规范名）/270（注册调用）；“239”无出处不采用 | 第五轮决策 3 |
| 决策 4 RW-14/RW-16 选型 | Inspector 以 PropertyGrid 为主干 + RW-14/Y-3 缝；命令面板过滤留宿主模型层 | 第五轮决策 4 |
| 矩阵重估 | 15 格重估为 13 `unverified` / 2 `build-verified`，校验脚本 PASS | 第五轮决策 5 |

## 1. 宿主侧任务（H 系列）

### 1.1 批次 A：即改即消（无上游依赖，建议优先）

依据：feedback-20261101.md 第二轮 R4（H-1..H-8 表）与第三轮 T7（H-10..H-17 表）。这些改动都不新增语义，只把自研平行路径换成已存在的上游能力。

| ID | 任务 | 验收标准 | 关联/先决 |
|---|---|---|---|
| H-1 | `prune_controls` 改用 `ApplyReport::removed_ids`（`view/apply.rs:275`），删除 tree_index 路径反推 | 预览服务里旧 id 回收走 `removed_ids`；C-1 旧绕行整段删除；相关测试保持绿 | 上游能力已就绪 |
| H-2 | 工具栏 i18n 刷新改为 `set_item_text`/`item_mut` 原地改文本 | `bar.clear()`+逐项重插消失；action id 与状态保留 | 上游能力已就绪 |
| H-3 | View/Panels 子菜单改用 `MenuHandle::new_submenu`/`row_rect` 嵌套渲染 | 子菜单不再扁平化；现有菜单测试保持绿 | 上游能力已就绪 |
| H-4 | Inspector mixed 行改用 `PropertyItem::mixed`/`with_placeholder` | mixed 值显示为占位而非只读拼接文本 | 上游能力已就绪 |
| H-6 | 主题读取改 `ThemeManager::theme_snapshot()` 单次取锁 | 双锁防死锁注释与绕行退役；无行为变化 | 上游能力已就绪 |
| H-7 | 自写 offset/position 换算收敛到 `CodeEditorDocument::offset_of/position_of` | 仅编辑器文档调用点收敛；纯 `&str`/DAP 列语义保留 | 上游能力已就绪 |
| H-8 | `Catalog::fill` 改用 `interpolate_into` | 自研 `{name}` 填充删除；GUI 路径行为不变 | 上游能力已就绪 |
| H-12 | 宿主 `Bounds` 透传 schema 的 `step/places` | 属性编辑器数值步进/精度按 schema 生效 | 上游 B26.02 已就绪 |
| H-13 | 删除重复的 `register_property_defaults` 调用 | suites install 后不再二次注册；默认值行为不变 | 上游能力已就绪 |
| H-14 | `workbench_gui.rs:2120-2122` 改用 `TabWidget::tab_index_at`（handle 已有） | 手写命中计算删除；标签页右键菜单行为不变 | 其余 24 处 `with_widget_mut`/14 处 downcast 收编 ⇢ RW-63 |
| H-15 | `open_url.rs` 改消费上游 `open_external`（`traits.rs:2400`） | 自研打开 URL 路径删除；Linux/macOS/Windows 行为一致 | 原则 #59 |
| H-16 | `locale.rs` env 解析收敛到上游 `system_locale` | 自研探测删除或仅保留上游未覆盖的 `zh-Hant` 拒绝规则 | 原则 #59 |
| H-25 | 退役 C-7 绕行，改用 `load_theme_named`（`theme/manager.rs:428`） | 主题登记不再“先激活再恢复”；返回文件内主题名 | 上游已提供 |
| H-17 | FlexLayout 预览前用 `items_mut()` 回填 `ChildInfo.hints.max` | 预览与生成的 max 约束一致 | 配合 RW-49 |

### 1.2 批次 B：面板与体验（中等工作量）

依据：feedback-20261101.md 第三轮 T7/T16.1。

| ID | 任务 | 验收标准 | 优先级 |
|---|---|---|---|
| H-18 | Inspector 颜色属性接上游颜色族四件套（`color_picker`/`color_well`/`color_history`/`swatch`），替换 hex 文本框（`property_editor.rs:142-148`） | 颜色属性可视化编辑；hex 仍可手输 | **P1（最高）** |
| H-19 | 画布拖拽接 `DragGhostLayer` | 拖拽有可视 ghost，取消/落下正确清理 | P2 |
| H-20 | 接 `tooltip_manager` 全局管线 | 控件/条目 tooltip 可显示（与 RW-62 上游渲染缺口区分：控件级先可用） | P2 |
| H-21 | 资产面板接 `image_gallery` + `thumbnail_cache` | 资源面板有缩略图与缓存 | P2 |
| H-22 | 主题 `Motion` token 编辑接 `bezier_curve_editor` | 缓动曲线可视化编辑 | P2 |
| H-23 | 主题编辑器暴露 `ControlMetricsProfile`（Material/Macos/Cupertino） | 切换度量档后控件尺寸档变化 | P2 |
| H-24 | 预览切主题接 crossfade（`set_theme_faded`/`tick_theme_crossfade`） | 主题切换有淡入淡出且尊重 reduced-motion | P3 |
| H-5 | 5 处列表面板 clear+全量重插部分退役（限于上游无逐行 set text） | 数量相等时跳过；差异删插；注释如实说明限制 | P2 |
| H-10 | 消费上游 `unwired_events`，在生成/检查报告中提示未接线信号 | 未接线信号可被设计器呈现（原则 #55） | P2 |
| H-11 | 消费 `GenerationReport.icon_tokens` | 生成报告展示图标 token | P3 |

### 1.3 批次 C：cookbook 集成模式借鉴（H-26..H-35）

依据：feedback-20261101.md 第四轮 T16.2。每条先做可行性验证，再决定是否排产。

| ID | 模式 | 上游示例 | 优先级 |
|---|---|---|---|
| H-26 | 离屏控件普查 → 工具箱缩略图 + “主题盲控件”告警 | `examples/control_rendering_census.rs` + `widget/census.rs:299` | P2 |
| H-27 | SVG 快照导出 → 面板/文档离线预览资产 | `examples/export_control_svgs.rs` | P2 |
| H-28 | Harmony NAPI 桥接入（宿主脚手架 0 引用） | `examples/harmony_napi_bridge_flow.md` + `rw.h` | P2 |
| H-29 | 声明式自适应视口 → 一份文档三档断点预览 | `examples/view_adaptive.rs` | P2 |
| H-30 | 主题解析转储 → “主题值→解析值”对照列 | `examples/theme_resolve_dump.rs:33` | P3 |
| H-31 | 图标普查+图集 → 图标选择器清单 | `examples/icon_census.rs` | P3 |
| H-32 | 主线程菜单/快捷键探针 → 菜单接线验收 | `examples/menu_shortcut_runtime.rs` | P3 |
| H-33 | 原则测试镜像（单一创建机制/统一属性 API） | `examples/control_creation_is_single_mechanism.rs` 等 | P3 |
| H-34 | 嵌入式宿主形状 → headless/缩略图取帧 | `examples/embedded_host.rs` | P3 |
| H-35 | 呈现路径 smoke → 窗口级前置检查 | `examples/present_smoke.rs` | P3 |

## 2. 既有缺陷（VCS 系列）

| ID | 缺陷 | 复现 | 失败模式 | 排查方向 |
|---|---|---|---|---|
| VCS-1 | `tests/vcs_conflict_regenerate.rs` 2 个用例在 Linux 上失败：`a_generator_gap_is_not_reported_as_a_successful_resolution`（:374）、`a_stale_resolution_stays_incomplete_and_the_previous_content_is_recoverable`（:321） | `cargo test --test vcs_conflict_regenerate`（Linux x86_64；stash 掉 probe 常量改动后同样失败，与第五轮改动无关） | stderr 报 `Translation directory 'language' could not be read`；stdout 出现 `shell.error.prefix (has no text)`，断言“drift is named”找不到文案 | 测试进程 cwd 在 `/tmp/rustage-vcs-conflict-…`，`language/` 按启动目录解析失败；检查 `RUSTAGE_LANGUAGE_DIR` 回退链与错误文案的 i18n 缺失路径，断言应比较键而非渲染文案或测试需注入语言目录 |

说明：该缺陷在 2026-11-01 macOS 记录中未出现（`log-20261101-2.md` 的 136 passed 套件不含此集成测试），疑为 Linux/环境相关；修复前不得把该套件写成绿。

## 3. 验证补证（M 系列，完成后回写 support-matrix）

| ID | 补证项 | 命令/条件 | 回写目标 |
|---|---|---|---|
| M-1 | macOS 五 profile `--lib` 构建（3.2.1） | `cargo build --manifest-path <rust-widgets>/Cargo.toml --no-default-features --features <profile> --lib`（macOS aarch64） | `macos × tablet/mobile/mini/embedded` 四格由 `unverified` 升级 |
| M-2 | macOS GUI 有界会话（3.2.1） | `--workbench-gui --exit-after-ms 4000` 真实窗口 | `macos` 宿主由 `build-verified` 恢复 `certified` |
| M-3 | Windows xwin 交叉构建（3.2.1） | rust-lld + xwin 的 `--bin rustage` 与五 profile `--lib` | `windows` 行五格升级 |
| M-4 | Linux `linux-native`（GTK）构建与 GUI 运行（3.2.1） | `cargo build --features linux-native` + 窗口会话 | `linux` 宿主由 `build-verified` 评估 `certified` |

当前矩阵基线（2026-11-03，4.0.0）：10 `unverified` / 5 `build-verified`（macOS 宿主 `certified`；校验脚本 PASS）。补证必须附命令、环境、日期，不拿旧修订证据冒充（矩阵规则 2/4）。

## 4. 上游请求（RW 系列，blocked upstream）

共 100 条，证据与修法建议全部见 feedback-20261101.md 对应编号（本表只登记标题与宿主阻塞点）。上游未关闭前，相关宿主任务标注 `blocked`，不自行实现替代语义（原则 #25）。

### 4.1 P1（23 条，阻断 roadmap 验收或迫使宿主手写绕行）

| ID | 标题 | 宿主阻塞点 |
|---|---|---|
| RW-1 | DockArea 不是 Widget，无法挂进窗口 | F04 停靠区装配 |
| RW-5 | 列结构增删/改名无 API 无信号 | 数据网格面板 |
| RW-7 | TableWidget 不吃 IncrementalTableDataSource | 大表/虚拟化 |
| RW-9 | 集合模型不可绑定，ObservableList 无控件消费 | F13 数据绑定 |
| RW-19 | 片段会话 API | 编辑器片段 |
| RW-20 | 结构选择扩展点 | 编辑器结构选择 |
| RW-21 | inlay hints / CodeLens | 编辑器 |
| RW-25 | 跨字段规则无接缝 | 表单/Inspector |
| RW-26 | 异步校验与过期取消 | 表单校验 |
| RW-29 | 命中测试无节点身份 | 画布选中 |
| RW-30 | 覆盖层/装饰绘制钩子 | 画布（Y-1 缝） |
| RW-41 | 父子类型校验契约缺失 | T09.04 子节点约束 |
| RW-42 | 生成物无逐节点锚点 | T15.03 source-map |
| RW-43 | JSON loader 未接 ViewEngine（G-406） | 编辑-重载增量 |
| RW-48 | solver 级钳制/溢出遥测缺失 | T10.03 布局诊断 |
| RW-52 | 窗口激活 API 与窗口焦点触发器缺失 | `window.*` 命令族 |
| RW-53 | 关闭 veto 缺失 | 未保存修改拦截 |
| RW-56 | 日期类控件无时钟注入钩子 | T14.06 跨机器基线 |
| RW-61 | 右键菜单策略不一致 + CodeEditor 内置菜单双轨 | 统一右键策略 |
| RW-67 | 主力四图表数据无属性载体 | T27.07 图表编辑 |
| RW-88 | macOS 像素滚动增量当槽计数 | macOS 画布滚动手感 |
| RW-89 | Windows/Linux 横向滚轮分量被丢 | 画布/时间线横向平移 |
| RW-96 | 运行时语言切换无失效机制 | 设计器语言切换 |

### 4.2 P2（39 条）

RW-2（StatusBar/ToolBar 嵌入只收 Box<dyn Widget>）、RW-3（ToolButton 无 tooltip）、RW-6（ColumnSpec 无数据类型）、RW-8（无公开内存表模型）、RW-10（树项无图标）、RW-11（PropertyGrid 无差量行更新）、RW-12（PropertyGrid 无行级错误）、RW-15（CommandPalette 条目无 enabled）、RW-17（集合控件无空态/错误态）、RW-22（书签生命周期）、RW-23（导航栈）、RW-27（类型化校验通道）、RW-31（模型空间网格）、RW-32（标尺与对齐参考线）、RW-34（A11yRole 无 name）、RW-36（IconName 缺 19 个语义名）、RW-38（i18n 无逐语言目录访问）、RW-39（inject_translations 无热重载路径）、RW-44（隐藏 setter 面未收编）、RW-47（layout schema 未接生成器）、RW-49（apply_layout_with_context）、RW-54（窗口位置 get/set）、RW-57（整树断言工具）、RW-58（通用 scroll 原语）、RW-62（条目级 tooltip 永不渲染）、RW-63（handle 平价 Top15）、RW-64（SaveState 未实现清单）、RW-65（a11y 表 132/230 Unknown）、RW-66（信号命名混乱）、RW-68（图表数据不进 data_binding）、RW-69（数据变更不可观测）、RW-70（Heatmap 数据无属性）、RW-71（RadarChart axes/series 无属性）、RW-81（媒体三伴件未注册）、RW-82（VideoPlayer 缺 source 属性）、RW-90（无 debounce/throttle）、RW-91（无 queued connection）、RW-92（Tab 焦点不按窗口限定）、RW-97（无 BCP-47 回退链）。

### 4.3 P3（38 条）

RW-4（TabWidgetHandle 便利层）、RW-13（Number 无边界参数）、RW-14（cell_changed 只带 row/text；Inspector 选型已定，见决策 4）、RW-16（CommandPalette 无动态 provider；过滤归属已定，见决策 4）、RW-18（懒加载失败无消息位/TreeTable、ListView 无编辑）、RW-24（环绕选择/inline diff）、RW-28（无取消信号）、RW-33（光标形状/平移手势）、RW-35（无逐控件 role 覆盖）、RW-37（图标 RTL 镜像）、RW-40（方向映射/复数类别）、RW-45（bindings_of/发射计数）、RW-46（verify_target 无离线通道）、RW-50（Absolute 无 by-id 改位）、RW-51（Grid 无分数单位）、RW-55（IME/Linux text_scale）、RW-59（基准缺口）、RW-60（上游文档漂移）、RW-72（图表动画无进度 setter）、RW-73（BarChart 多序列无增量）、RW-74（LineChart 点级无增量）、RW-75（ChartDataTable/ZoomStrip attach 未上移）、RW-76（Gantt 任务无差量 CRUD）、RW-77（ChartCrosshair 未注册）、RW-78（Volume/Indicator 无流式入口）、RW-79（QuoteBoard 无属性/增量）、RW-80（OrderBook book 无属性）、RW-83（Lottie/AnimatedImage 缺 current_frame）、RW-84（AnimatedImage 无文件加载）、RW-85（Rive 无 progress setter）、RW-86（AudioVisualizer 无样例数据）、RW-87（ChapterListPanel 无差量）、RW-93（FocusPolicy 未接运行时）、RW-94（无信号级整体暂停）、RW-95（手势引擎只吃 Touch）、RW-98（占位符无类型化/本地化格式化）、RW-99（token 编辑不发 on_theme_changed）、RW-100（drum_column/preset_chip_bar/thumbnail_cache 未注册）。

### 4.4 扩展缝与新 kind（随 RW 一并提给上游）

- **Y-1..Y-7 扩展缝**：canvas overlay、可编辑列表、PropertyEditorDelegate、CompletionSource、SyntaxHighlighter、行号槽、TerminalView 喂输出（优先于新 kind）。
- **新 kind 唯一强建议：NodeGraph**（F13 事件连线可视化；最小 API/信号/profile 见 feedback 第四轮 T4）。
- 上游确认不存在但现成控件+扩展缝可覆盖：JSON 树查看器、表达式输入、约束编辑控件、独立标尺（并入 RW-32）、CSS 检查器、状态机图。

## 5. 依赖与建议执行顺序

**上游依赖链（宿主任务先等上游）**：
- H-14 的剩余收编 ⇢ RW-63；H-4 的更完整形态 ⇢ RW-14；H-5 的完全退役 ⇢ 上游逐行 set text；H-20 的条目级 tooltip ⇢ RW-62；H-12 的完整精度 ⇢ RW-13；H-10 的接线覆盖面 ⇢ RW-45。
- 依赖已就绪、可立即动的：H-1、H-2、H-3、H-6、H-7、H-8、H-12（基础字段）、H-13、H-25、H-15、H-16、H-18、H-19、H-21、H-22、H-23、H-24。

**建议顺序**：
1. **批次 A**（1.1，全部即改即消，无上游依赖）——先关闭 H-1..H-8/H-12..H-16/H-25。
2. **VCS-1** 与 **M-1..M-4** 可与 A 并行（互不改同一文件：VCS-1 在 `tests/`，M 在构建验证）。
3. **批次 B**（1.2）——按 H-18 → H-19/H-20/H-21 → H-22/H-23/H-24 排。
4. **批次 C**（1.3）——逐项可行性验证后再排产。
5. **RW 系列**：按 4.1 → 4.2 → 4.3 向上游提；每次上游升级后重扫本表状态并回写 feedback。

**统一验收**：每条任务完成后回写 (a) 本表状态、(b) feedback 对应编号的关闭证据、(c) roadmap 任务行（如适用）、(d) 涉及矩阵的写 support-matrix JSON+MD 并跑校验脚本。

## 6. 覆盖与边界

- 本文不重复扫描证据；所有 file:line 见 feedback-20261101.md（四轮）与第五轮决策记录。
- 不修改上游 rust-widgets 代码；RW 条目只作为请求提交。
- 不修改宿主既有属性/方法/变量名，除非确有冲突（原则 #34）。
- 不新增自造控件：宿主侧任务一律优先消费上游能力（原则：控件全部来自 rust-widgets）。
- 已知未覆盖：`benches/` 性能实测、Windows/macOS 实机运行、GPU 呈现路径运行行为；这些属 M 系列扩展，见 support-matrix 规则 2/4。

## 7. 执行状态（2026-11-03）

证据与验证命令见 [log-20261103-1.md](../log/log-20261103-1.md)。

| 组 | 已关闭 | 未完成 / 依据 |
|---|---|---|
| 批次 A | H-1、H-2、H-3、H-4、H-6、H-7、H-8、H-12、H-15、H-16、H-17、H-25；H-13/H-14 复核为已满足 | 无 |
| 批次 B | H-5、H-10、H-11、H-18、H-19、H-21、H-22、H-23、H-24、H-20 | 无（H-21 前置资产面板、H-22/H-23 前置主题面板已装配） |
| VCS | VCS-1 | 无 |
| M | M-1（macOS 四 profile）、M-2（GUI 有界会话） | M-3：上游 RW-101 阻断；M-4：本机无 Linux GTK 环境 |
| RW | 新增 RW-101（4.0.0 Windows 编译回归，P1） | 其余 RW 仍待上游 |

**矩阵（JSON+MD 同步，校验 PASS：10 `unverified` / 5 `build-verified`）：** macOS 宿主恢复
`certified`，macos × 五 profile 均 `build-verified`；windows 与 linux 的 4.0.0 证据均未建立，
维持/降为 `unverified`。
