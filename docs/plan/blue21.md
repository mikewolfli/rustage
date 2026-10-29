# blue21｜全项目深度扫描报告（2026-10-30）

> 依据：[principle.md](./principle.md) 为评审准则，对照 [roadmap.md](./roadmap.md) 逐任务核对，
> 并对全部 `src/` 做控件复用、隐藏问题、效率、i18n 四路专项扫描。
> 基线：宿主单 crate（351 个 `.rs`，约 22.3 万行），依赖上游 rust-widgets 锁定
> rev `d44ca0e9003365282815f586bdbf152dcebf749a`（3.1.0，`Cargo.toml:28,32`）；上游本地检出
> HEAD 为 `c2d7280f`（main，比锁定版多 3 个提交 `891636ac/4cc20127/c2d7280f`，主要改 code_editor）。
> **本轮为只读扫描**：不修改任何代码；不运行构建/测试；全部结论附 文件:行 证据。
> 标注 ⚙ 的条目由主审计逐行复核过源码原文；其余条目来自并行静态扫描，证据行号已随文给出。
> 涉及上游 rust-widgets 的修改建议已汇总到 [../rw-feedback/feedback-20261030.md](../rw-feedback/feedback-20261030.md)。

## 结论摘要

| 章节 | 条数 | 最重要发现 |
|---|---|---|
| task | P1×6 域、P2×10 | F25 发行域最薄弱（2/9）；§2/§3 账目与 GUI 装配差距大；4 项 roadmap 声明"复用库"实际自研 |
| widgets | 违规 4+2 | 命令面板/工具条/标签页/属性行为手工拼装，上游有现成控件；布局契约未分叉 ✅ |
| issues | P1×3（约15处）、P2×15 | `let _ =` 吞 join/stop/回滚 Result 是最大可诊断性缺口（原则 #22） |
| optimized | 高×5、中×6、低×7 | 拖拽 O(n²) + 每指针事件全量快照 + 每帧整快照克隆是"丝滑"最大障碍 |
| language | 1 个结构性偏差 | 语言目录硬编码在 Rust 源码（自研行式包，非 JSON）；GUI 完全未接 i18n |

---

## task｜Roadmap 对照：未实际完整实现的项目

方法：按 roadmap.md 行号区间提取 123 个 T 任务 + §2/§3/§4 账目，逐项在 `src/`、`tests/`、
`tools/` 取证；以代码为准，`traceability.md` 仅作对照，不照抄其声明。

### 覆盖率总览

| 域 | 结论 | 域 | 结论 |
|---|---|---|---|
| F00 | 9/9（1 项 P2 证据路径陈旧） | F14 | 7/8，1 部分（T14.08） |
| F01 | 8/9，1 部分（T01.02） | F15 | 8/8 |
| F02 | 11/11 | F16 | 10/11，1 部分（T16.11） |
| F03 | 8/8 | F17 | 8/8 |
| F04 | 8/8 | F18 | 8/8 |
| F05 | 7/8，1 部分（T05.02） | F19 | 8/8（T19.09/10 为计划内未来项） |
| F06 | 10/10 | F20 | 9/10，1 部分（T20.08） |
| F07 | 10/10 | F21 | 9/10，1 部分（T21.07） |
| F08 | 8/8 | F22 | 9/10，1 未找到（T22.07） |
| F09 | 7/8，1 部分（T09.02） | F23 | 8/12，3 部分（T23.07/11/12） |
| F10 | 8/8 | F24 | 6/10，3 部分，1 未找到（T24.07） |
| F11 | 6/8，2 部分（T11.03/04） | F25 | **2/9，2 部分，5 未找到（最薄弱域）** |
| F12 | 9/9 | F26 | 8/8 |
| F13 | 8/9，1 部分（T13.02） | F27 | 8/8 |
| §2/§3 | 部分（账目缺口大，见下） | F28 | 8/8 按计划未实现（正确，非缺口） |
| §4 | 基本完整，1 存疑 | §34/§35/§37 | 完整/有代码支撑 |

全局：`todo!`/`unimplemented!`/`FIXME` 全仓 0 处；164 处 `#![allow(dead_code)]` 为 CLI 骨架期统一
约定且各域经 `src/app/run/*.rs` 调度接通（如 `run/preview.rs:13`、`run/debug.rs:20`），非孤岛代码。

### P1 缺口（缺交付项）

| 任务ID | 名称 | 判定 | 缺口描述 | 证据(文件:行) |
|---|---|---|---|---|
| T05.02 | §3 菜单树 | 部分 | 机制完整（唯一 ID、problems 校验），但注册命令仅约 13 个、GUI 菜单仅 File/View/Layout/Editor 4 组，远低于 §3.2 约 70 组叶命令总账 | ⚙ `src/app/commands/command.rs:798-900`；⚙ `src/app/run/workbench_gui.rs:1525-1552`；`src/app/commands/menu.rs:222-270` |
| T11.04 | 设计令牌与主题 | 部分 | roadmap 声明主题读写复用 `ThemeManager::load/save_theme`，全仓无 ThemeManager 引用；`design_tokens.rs` 自研内存 TokenSet，无主题文件读写 | `src/app/style/design_tokens.rs:160-251`；上游 `rust-widgets/src/theme/manager.rs:455` |
| T11.03 | CSS 检查器 | 部分 | roadmap 声明检查器消费 `css_trace`，全仓无 css_trace 引用，`css_view.rs` 自研解析/特异性计算 | `src/app/style/css_view.rs:50-122`；上游 `rust-widgets/src/style/mod.rs:54` |
| T13.02 | 类型化绑定 | 部分 | roadmap 声明复用 `Binding<T>`/`BindingPipeline`/`bind_widget_property`，全仓无引用，自研 Conversion/Graph；`report.rs` 已如实登记 not consumed；验收由自研测试达成 | `src/app/quality/report.rs:538-539`；`src/app/data/binding.rs:1-30,651-808` |
| T01.02 | 平台服务适配 | 部分 | roadmap 声明剪贴板复用 `RichClipboardBackend`（HTML/RTF/图片/文件列表/自定义 MIME），实际仅读路径做富内容分类，写路径仅纯文本（延期至 T05.06） | `src/host/clipboard.rs:22-24,99-115` |
| T25.02/T25.04/T25.05/T25.06/T25.08 | 发行域 5 项 | 未找到 | 三桌面打包/签名/公证、无网安装边界/卸载、RC 全旅程复验、发布教程/已知限制手册、维护分支/升级演练均无实现（docs/ 无发行手册；tools/ 无 installer/dmg/msi/deb） | `src/app/release/mod.rs:1-14`（仅 manifest/lockfile/update_journal） |
| T23.11 | 受控外部工具/MCP 接缝 | 部分 | 仅角色工具白名单；无 MCP 协议接缝、调用预览、逐项授权（P0 级任务） | `src/app/ai/workflow.rs:413-427`；全仓 grep "MCP" 0 命中 |
| T23.12 | 双 agent 会话治理 | 部分 | 候选摘要+修订绑定已实现；会话恢复/交接/并发冲突/材料清理未找到（P0 级任务） | `src/app/ai/workflow.rs:280-358` |
| §2.2/§3.3/§3.4 | 面板/上下文菜单/向导账目 | 部分 | §2.2 约 41 面板 vs GUI 装配 8 个（+shell 内置 4，`messages` 不在账目）；§3.3 的 13 类上下文菜单仅模型级测试、GUI 无右键菜单；§3.4 的 11 个必需向导无统一外壳（仅新建工程+一个通用壳） | ⚙ `src/app/run/workbench_gui.rs:1393-1412`；`src/app/commands/focus.rs:362,400`；`src/app/quality/wizard.rs:138`；`src/project/lifecycle/create.rs:18-24` |

### P2 偏差（文档/实现轻微不符）

| 任务ID | 判定 | 缺口描述 | 证据(文件:行) |
|---|---|---|---|
| T09.02 | 部分 | 声明运行侧查询复用库 `TreeIndex`，实际自研 `SceneIndex`；原因有据（上游 `TreeIndex::build` 为 `pub(crate)`），scene.rs 已登记该库缺口（详见 widgets 章节 W-6） | `src/app/canvas/scene.rs:30-43,201`；上游 `rust-widgets/src/view/identity.rs:185` |
| T04.02 | 部分 | "未实现服务不显示假面板"的门（`is_available`）在 GUI 装配路径被绕过：services 集合构建后以 `let _ = &services;` 丢弃，`show()` 不校验可用性 | ⚙ `src/app/run/workbench_gui.rs:1393-1397`；`src/app/shell/panels.rs:230` |
| T05.08 | 部分 | audit 门已接 CLI，但"平台快捷键及入口一致性测试"无平台分支实现 | `src/app/commands/audit.rs:79`；`src/app/commands/shortcuts.rs` |
| T00.01 | 完整(边缘) | roadmap 引用的证据 `docs/log/log-20261029-11.md` 实际在 `docs/log/archived/` 下，路径陈旧 | `Cargo.toml:28`；`roadmap.md:474` |
| T01.07 | 完整(边缘) | `ProjectServices` 注册/关闭顺序完整，但 app 侧消费者仅 readiness 探针，GUI/CLI 启动路径未接入 | `src/host/registry.rs:216`；消费者仅 `src/host/readiness.rs:46` |
| T14.06 | 部分 | 条件记录/NotComparable 达成；"对齐库侧字节级 SVG 快照"在宿主无引用（自建像素基线） | `src/app/preview/visual_baseline.rs:4-24` |
| T14.08 | 部分 | 100 次绑定/重建/多视口负载验收：场景定义齐备，仓库内无实测产物（§35.2 自述待 MS00 校准） | `docs/plan/benchmark.json:58,150,176`；`tools/benchmark/check-benchmark.test.mjs:120` |
| T16.11 | 部分 | headless CLI 验证已实现；"CI 配置可导出"交付项无代码 | `src/app/build/headless.rs:6-7,81` |
| T20.08 | 部分 | 冲突→重新生成链路有真实 git 集成测试；外部变化与合并中断专项用例未见 | `tests/vcs_conflict_regenerate.rs:86-126` |
| T21.07 | 部分 | 证据包已实现；故障注入专项 harness 与 100 轮泄漏测试未定位 | `src/app/quality/report.rs:1202-1218` |
| T22.07 | 未找到 | 扩展开发文档/示例/兼容测试缺失 | docs/、examples/ 目录清单 |
| T23.07 | 部分 | 预算与脱敏已实现；断流/取消/耗时/费用记录未找到 | `src/app/ai/workflow.rs:112-166,409-423` |
| T23.09 | 部分 | `.ai` 路径越界拒绝已实现；符号链接专项诊断未见 | `src/app/ai/workflow.rs:209,489` |
| T24.02 | 部分 | define_theme 支持 light/dark；高对比主题未见 | `src/app/style/design_tokens.rs:430-437` |
| T24.05 | 部分 | IME preedit smoke 探针+剪贴板已实现；组合输入/候选位置/多屏切换无实测记录 | `src/host/platform.rs:115`、`src/host/startup.rs:514` |
| T24.07 | 未找到 | 新手引导/工程教程/命令发现：grep onboard/tutorial 0 命中 | 全仓搜索 |
| T24.10 | 部分 | 目标应用 i18n 消费库 TranslationFile 已实现；宿主侧键提取/伪本地化工具未找到 | `src/app/run/target_i18n.rs:4-13` |
| T25.03 | 部分 | 更新状态机+备份+中断持久化已实现；CLI 确认/中断/恢复接线自述 separate change 未完成 | `src/app/release/manifest.rs:26-27`、`src/app/release/update_journal.rs:7-16` |
| T25.07 | 部分 | 冻结校验 verify_against 与 SHA-256 已实现；签发/下载后校验流程未接 | `src/app/release/manifest.rs:12-17` |

### ⚠️ 需人工确认（不猜测，等待结论）

1. **上游 API 核对基准**：本表对上游 API（ThemeManager/css_trace/TreeIndex pub(crate)）的核对基于
   上游检出 HEAD `c2d7280f`，宿主钉的是 `d44ca0e`（3.1.0）；两者间 3 个提交只改 code_editor，
   但 4 个"复用未落地"缺口的库侧前提建议在 `d44ca0e` 上复核后坐实。
2. **F25 是否属"里程碑未到"**：roadmap.md:950 T25.05 前置为 T19.08/T23.08；若按 MS 顺序推进，
   F25 的 5 个"未找到"可能是时序正常而非遗漏——请对照 §37.2 里程碑状态判定后再定优先级。
3. **T04.02 services 门绕过是否有意**：`workbench_gui.rs:1393-1397` 构建 services 后丢弃（`let _ = &services;`），
   可能是有意简化（8 个内置面板的服务模型恰好都存在），但与 T04.02 验收机制不一致。
4. **§4.1 SharedArtifact 语义等价性**：roadmap 要求的 `SharedArtifact`（不变快照/评审绑定候选）无同名类型，
   `src/app/ai/workflow.rs:265,360,406` 以 Session/Candidate/ChangeSet/Verification 拆散承载，语义是否等价需人工判定。
5. **T14.08/T21.07 实测证据是否在仓库外**（`target/bench/`、`docs/log/` 早期文件）本轮未穷尽核对。
6. **T02.04 多窗口入口**：多视图窗口在 F04 已实现（`src/app/shell/windows.rs:4-6`），工程树侧入口是否齐备未逐项核对。

---

## widgets｜控件复用审计（全部控件应来自 rust-widgets）

范围：宿主 345 处 `rust_widgets::` + 2 处 suites 引用；上游注册表共 270 个 `self.register(`
（`rw/src/widget/capability/registration.rs`）；宿主开 `desktop` feature → 全部控件参与编译。

### W-1 面板↔上游双向差异：0 违规 ✅

- 面板不是手写清单：`Catalog::from_contract_at` 直接枚举 `factory.constructible_names()+widget_names()`，
  未知 id 被拒绝而非造条目 —— `src/app/canvas/widget_catalog.rs:460-496`、`src/app/run/design.rs:24,33-38`。
- 非构造名（child-only 如 MenuItem）也收录并标记 unavailable，不静默丢弃 —— `widget_catalog.rs:469-482`。
- suite 控件 `form`/`form_set` 经宿主工厂（上游同仓库子包 rust-widgets-suites）入面板，不算自造 ——
  `src/app/contract.rs:36-42`、`Cargo.lock:2464-2470`。

### W-2 重复造轮子（违规清单，应替换为上游控件）

| # | 严重度 | 证据 | 问题 | 上游现成控件 |
|---|---|---|---|---|
| W-2a | 高 | ⚙ `src/app/run/workbench_gui.rs:2788-2801`（另 1023,1069,1112） | 命令面板用 Panel+Label+LineEdit+ListBox 手工拼装，自写过滤/执行逻辑 | `CommandPalette`（`rw/src/widget/special_widgets/command_palette.rs:161,212`；`capability/constructors.rs:518`） |
| W-2b | 中 | ⚙ `src/app/run/workbench_gui.rs:1560-1588` | 工具条 = 11 个裸 `new_button` + 响应式均分；同文件菜单条却用了上游 MenuBar(:1519)，风格分裂 | ToolBar/ToolButton（宿主 `src/app/shell/toolbar.rs:67,357-374` 已证明可用） |
| W-2c | 中 | `src/app/run/workbench_gui.rs:2668-2685,1335-1351,253-260` | 标签页 = 每 tab 一个 Button + `set_widget_visible` 手动切页 | TabWidget/TabBar（`rw/src/widget/kind.rs:172,316`；`constructors.rs:895,204`） |
| W-2d | 中 | `src/app/run/workbench_gui.rs:1747-1763,1881-1895` | Inspector/Data 属性行 = Panel+LineEdit+Apply/Clear/Reset 手工拼装（校验逻辑是契约驱动的模型层，仅表现层重复） | PropertyGrid（`rw/src/widget/kind.rs:275`；`constructors.rs:1704`） |
| W-2e | 低 | ⚙ `src/app/run/workbench_gui.rs:1658,1662` | `panel.set_title(title)` 后又放一个同名 Label —— 上游 Panel(GroupBox) 自绘标题带，疑似双标题（需运行核实 set_title 是否在该后端渲染） | 上游标题带（`rw/src/widget/container_widgets/groupbox.rs:887-930`） |
| W-2f | 低 | `src/app/canvas/scene.rs:36-43,200-212` | `SceneIndex` 复刻 `TreeIndex` 的 path/key 规则（模块文档自认是"等待上游公开构造器"的适配器，已声明待移除） | `TreeIndex`（上游 `view/identity.rs:185`，`build` 为 pub(crate) → 见 feedback E-1） |

### W-3 合法项（排除，不属违规）

- `draw_canvas` 走上游 Canvas 控件（`workbench_gui.rs:823-860`）；`CanvasLayout`/`WorkbenchLayout`
  实现上游 `Layout` trait 并接入 `WindowHandle::set_layout`（`workbench_scene.rs:434-489`、
  `workbench_layout.rs:16-17,284-297`、`workbench_gui.rs:217`）—— 画布选择框/参考线/设计空间映射
  属设计器变换，合法。
- 编辑器 = 上游 CodeEditor 适配器（`src/app/editor/adapter.rs:4-10`）。
- 图标：用上游 IconName/IconSource，仅上游 69 个内置图标名缺失的语义图标（bug/canvas/component/
  flow/inspect/layout/theme/library）才出静态几何（`src/resources/icons.rs`、`generated_host_icons.rs:197-470`；
  上游 `rw/src/widget/display_widgets/icon_names.rs:176` 起）。

### W-4 结构性注意

1. 上游契约不发布设计器分类，面板 category 只能按 kind 拼写分组 —— `widget_catalog.rs:258-265,696-698`
   （→ feedback E-3）。
2. **GUI 工作台尚无控件面板 surface**：只有 CLI `catalog` 命令与 Catalog 模型；GUI 面板清单
   pages/events/data/debug/outline/inspector/messages/problems 中无 palette —— `run/design.rs:20-26`、
   `workbench_gui.rs:73-78,1613-1655`（设计器核心交互面缺失，建议与 §2.2 账目缺口一并补）。

### W-5 布局契约核对（原则 #14）：未分叉 ✅

1. 设计文档布局：`LibraryEngine::solve` 把 min/max/preferred/grow/margin/cell/align 翻译为
   `Hints`/`LayoutParams`，放置委托上游 FlexLayout/StackLayout/GridLayout/AbsoluteLayout ——
   `src/app/layout/layout_plan.rs:64-68,924-1011`；Preview 与生成共用同一 engine（:567-572）。
   宿主自有算术仅 `container_for`（:1045-1094，无容器矩形时的外尺寸测量，非放置算法）。
2. 工作台 chrome：`WorkbenchLayout` 实现上游 `Layout`，region 切分调用上游 BorderLayout（`workbench_layout.rs:257-282`）。
3. 画布节点摆放：`CanvasLayout` 实现上游 `Layout` 全部四个入口（`workbench_scene.rs:434-489`）。

### W-6 上游反馈衔接

widgets 扫描产出的上游修改建议（W-2f/E-1..E-9）已全部登记到
[feedback-20261030.md](../rw-feedback/feedback-20261030.md)，此处不重复。

---

## issues｜新发现的问题与不完整功能

总评：无 P0。无 `todo!`/`unimplemented!`、无空函数体；15 处 unsafe 全部带 SAFETY 注释（工作规则 #4 ✅）；
SPDX 头 350/351。主要缺口集中在**静默丢弃 Result 的可诊断性**（原则 #22）与死门面 API。

### I-P1 静默失败（违反原则 #22，合计约 15 处）

| # | 证据 | 问题 |
|---|---|---|
| I-1 | `src/host/pty.rs:257`⚙、`src/host/pty_windows.rs:214`⚙、`src/host/subprocess.rs:734`、`src/host/lsp.rs:291,318`、`src/host/tasks.rs:980,998`、`src/app/run/gui_smoke.rs:358,362,377` | `let _ = reader.join()`/`handle.join()`：输出读取线程若 panic（如锁中毒），join 结果被丢弃，终端/子进程输出**静默停止**且无任何记录 |
| I-2 | ⚙ `src/app/run/build.rs:775` | `let _ = run.stop()`：注释声称 "must not leave the program running behind us"（:773），但 stop 失败被丢弃——承诺失效时子进程存活且不可诊断 |
| I-3 | `src/app/run/extension.rs:996`、`src/app/run/logic.rs:1508,1513-1514`、`src/app/run/build.rs:476,483` | `process.terminate`/`process.finish`/`executor.shutdown`/回滚 `active.import(false)` 的 Result 全部丢弃，清理失败不留痕迹 |

### I-P2 缺陷与瑕疵

| # | 类别 | 证据 | 问题 |
|---|---|---|---|
| I-4 | panic 放大点 | `src/host/lsp.rs:126,157`、`host/pty.rs:205,212,274,279`、`host/permission.rs:1101`、`host/log.rs:263,272`、`host/notify.rs:839`、`host/registry.rs:851,857`、`app/data/session_log.rs:148` 等（host+run 抽样约 14 处） | `lock().expect(...)`：与 I-1 叠加时，持锁线程先 panic → 毒化锁 → 后续每次调用 panic，链式崩溃 |
| I-5 | panic 风险（存疑） | `src/app/editor/snippets.rs:400` | `Carets::of(...).expect("the stops of one snippet are disjoint and ordered")`：不变量依赖 snippet 解析器自身正确性，位于用户展开 snippet 的输入路径；建议加注释或防御 |
| I-6 | 死门面 API | ⚙ `src/host/registry.rs:331` | `HostRegistry::prompt()` 全仓无消费者（唯一 `.prompt()` 命中是 `contracts/permission.rs:1634` 的 `PermissionEntry::prompt`，非同一类型）；`trust()`(:326)/`is_trusted()`(:336) 仅测试与 readiness 自检调用——真实信任路径走 `PermissionService`+`ProjectSession::decide`（`src/app/run/project.rs:60`） |
| I-7 | 注释漂移 | ⚙ `src/contracts/permission.rs:216-222` | 注释声称 "`Trusted` 仅由测试构造（T01.05 尚不存在）"，但 `trust_project()`（:345-346）与 CLI `--trust` 路径均已实现——陈述过时，会误导审计 |
| I-8 | 死代码 | `src/app/run/workbench_property.rs:429,438` | `accepted_tokens()`/`contract_default()` 已完整实现却无消费者（`#[allow(dead_code)]` 掩盖）；Inspector 声称的枚举选项/默认值展示未接线 |
| I-9 | 不完整功能 | `src/host/lsp.rs:38` | `Message.method` 标记 dead_code：server→client 请求的 method 字段无消费者，对 LSP 服务器主动请求的处理不完整 |
| I-10 | 能力落差 | ⚙ `src/host/network.rs:42-43,1126` | `TlsPolicy` 配置模型完备，但连接为 Plain TCP（"this build has no TLS client at all"，文档诚实）；配置面声明了当前无效果的能力 |
| I-11 | 能力落差 | ⚙ `src/app/shell/panels.rs:528-529` | outline 面板后端 document-tree 服务未实现，运行时诚实隐藏；但 GUI `workbench_gui.rs:1402` 仍对它调 `show` 且丢弃结果 |
| I-12 | 能力落差 | `src/host/dialog.rs:14` | 原生文件选择器未接入，驱动自绘实现（文档声明） |
| I-13 | 静默失败（GUI 启动） | `src/app/run/workbench_gui.rs:1388-1389,1408,1343,1435,1459,1520` | `panels.register/show`、`tabs.activate`、`layout.apply`、`attach_menu_bar` 的 Result 全部 `let _ =`；内置面板注册失败将静默消失（当前入参为静态常量，实际触发面小） |
| I-14 | 静默失败（合规工件） | `src/app/release/lockfile.rs:212` | `let _ = sbom.add(...)`：SBOM 组件添加失败静默，合规工件可能不完整而不自知 |
| I-15 | unsafe 隐患 | `src/host/pty.rs:96` | 使用 POSIX 标注 MT-Unsafe 的 `ptsname` 且无锁保护（Linux 有 `ptsname_r` 替代）；当前打开路径未见并发竞争证据，属隐患而非现行 bug |
| I-16 | 约定 | ⚙ `src/main.rs:1` | 全仓唯一缺 SPDX/版权头的文件（350/351 合规） |
| I-17 | 约定 | `src/project/lifecycle/create.rs:16`、`src/app/style/style_boundary.rs:19` | 注释级中文 2 处（前者引用中文 roadmap 原文，后者示例含 CJK 标识符；正文均为英文）。#63 基本达标 |
| I-18 | mod.rs 体量 | `src/app/run/mod.rs`（2208 行、82 个顶级项，含大量 parse_*/describe 辅助）、`src/cli/command/mod.rs`（1794 行，含 `parse_display`:508、`parse_size`:534 等业务解析） | 与"mod.rs 薄职责"约定（原则 #63）有张力；run/mod.rs 模块文档自辩"仅调度器+共享 helper"（:14-17） |

### 已查证排除项（非问题，避免后续重复排查）

- `src/app/run/workbench.rs:970,982` 的 `Display::new().expect` —— `cli/command/mod.rs:516-521` 已前置校验
  scale>0 且 milli 非零，用户输入不可触发。
- `src/contracts/store.rs` 10 处 expect 均有同级前置查找保护；`record/json.rs`/`page.rs` 的 expect 为解析器内不变量。
- `host/log.rs:264`、`locale/catalog.rs:423` 为文档化的最佳努力语义；`linux-native` feature 纯转发（`Cargo.toml:25`）。
- 运行路径 unwrap/expect 共 177 处/58 文件，抽样约 30 处均有前置不变量保护或为锁 expect。

---

## optimized｜效率与精炼改进（产品要求：高效、精炼、丝滑）

准则：原则 #23 —— 每帧绘制成本与视口相关；一次编辑的工作量与改动范围相关。仅记录，不改代码。

### 高优先级（最影响"丝滑"）

| # | 证据 | 模式与成本 | 建议方向 |
|---|---|---|---|
| O-1 | ⚙ `src/app/run/workbench_scene.rs:276-296` | `nodes()` 遍历全部节点时逐个调 `rect()`，而 `rect()` 对 `nodes: Vec<Node>`（:68）线性扫描 → O(n²)；调用点含拖拽 `mouse_moved` 回调（`workbench_gui.rs:2771-2772`，每像素一条事件） | node→rect 改 `BTreeMap<NodeId,Rect>`，或一次遍历直接产出 |
| O-2 | ⚙ `src/app/run/workbench_gui.rs:756-790` | `sync_canvas` 先对全部节点 `to_owned()` 名字 + O(n·m) 选中集判断（:764-767），**之后**才在 :785 检查 `canvas_scheduled` 去重——去重失效，每个指针移动事件都付 O(n) String 分配 | 把去重检查提到快照构建之前；快照只在 scene 版本号变化时重建 |
| O-3 | ⚙ `src/app/run/workbench_gui.rs:792-797` | 每帧整快照深克隆（`snapshot.clone()`，含全部名字 String）+ `Arc<Mutex>` | 存 `Arc<[...]>` 或版本号+双缓冲，避免逐帧深拷贝 |
| O-4 | `src/app/run/workbench_gui.rs:732-748` + `src/app/run/workbench_scene.rs:136-158` | 单点编辑全量重建 Outline：`rows()` 重建全部行字符串（每行 `"  ".repeat(depth)` 新分配），`list_box_clear_items`+逐行 `add_item`；`row_of`（:354-356）找一个行号走全树。调用点：select_node :890、apply_property :662、marquee :967、Reset :2026 —— 每次单点编辑 O(全文档) | 按 NodeId 定位行号缓存，增量更新单行；至少缓存缩进前缀与行号映射 |
| O-5 | ⚙ `src/app/run/workbench_gui.rs:845-849,822` | 每帧逐节点 `format!("{name} [locked]")`/`name.clone()`；`Font::new("Inter",13,..)` 每帧重建 | 标签在快照构建期算好；字体用 `const`/OnceLock 缓存 |

### 中优先级

| # | 证据 | 模式与成本 | 建议方向 |
|---|---|---|---|
| O-6 | `src/app/preview/preview_service.rs:646,529,576` | 每次活体编辑批 3 次整 Document 克隆；`View::build`（:213-230）每次更新重建整棵声明（模块文档 :601-605 自认） | 属性编辑走增量 patch；Document 用 Rc/COW；key 缓存为 String |
| O-7 | `src/app/preview/preview_service.rs:582-584,851` | 每次更新对全部节点 `install`，内部 `instances.iter_mut().find` 线性查 → O(n²) | `instances` 改 `BTreeMap<NodeId,Instance>`（`generations` 已是 map） |
| O-8 | `src/app/preview/preview_service.rs:194-197` | `apply_live` 逐编辑线性 `find` 节点，k 条编辑 O(k·n) | 维护 `BTreeMap<NodeId,usize>` 索引 |
| O-9 | `src/app/preview/preview_service.rs:548,689` | `view_ids` 只增不减，历史 control id 常驻；`live_controls` 只过滤不回收 → 长会话内存单调增长 | 更新时移除被替换 id，或定期压缩 |
| O-10 | `src/project/document/batch.rs:1267,1311-1316` | 每批应用整 ChangeSet 深克隆（`set.clone()`，ops 含 `Box<Node>`；:1270 还要做 inverse）；`store_of` 克隆 page 两次+全模型逐 op 重建，CLI 每次 `edit` O(全文档)（`src/app/run/project.rs:319,351`） | apply 按值接收 ChangeSet，inverse 从 ops 原地派生；文件→模型提供直接批量导入接口 |
| O-11 | `src/project/document/batch.rs:1028-1037` | `Plan::of` 的 staged 表逐 op 克隆属性全值做前镜像 | staged 只存版本号/引用，值按需回查 |

### 低优先级

| # | 证据 | 模式 | 建议方向 |
|---|---|---|---|
| O-12 | `src/app/preview/state_keep.rs:228,235,242-244,259` | `restore_focus` 两次 `index.keys()` 全量分配 + 线性 `find` | keys 查一次；present 建 `HashMap<NodeId,Present>` |
| O-13 | ⚙ `src/project/record/format.rs:2498-2508` | `check_unique` 用 `Vec::contains` 判重 O(n²) 且每项 `claim()` 分配 String | 改 `BTreeSet<&str>`（顺便消分配） |
| O-14 | `src/project/record/format.rs:2615-2619` | `check_generated_sources` 逐项线性 `pages.iter().any` | 预建 `HashSet` |
| O-15 | `src/app/canvas/scene.rs:254-259` | `SceneIndex::key_of` 线性反查（当前仅测试调用，未进热路径） | 若未来用于面板渲染，加 node→key 反向 map |
| O-16 | `src/app/run/workbench_property.rs:533-536` | `apply` 对每个选中节点 `self.name.clone()` | `editor.set` 改收 `&PropertyName` |
| O-17 | `src/app/run/logic.rs:551-567`、`src/app/vcs/repository.rs:661-708,2086,2237,2339` | 同步 `std::process::Command::output()`（cargo/git）阻塞调用线程（当前在 CLI 命令路径，不在 GUI 帧） | 保持 CLI 边界；进 GUI 时走工作线程+通知 |

### 合规亮点（无需改动，避免误改）

- `src/app/canvas/scene.rs:163,380-382`：SceneIndex 用 `OnceLock` 按 generation 惰性缓存，符合 #23。
- `src/contracts/store.rs:139-176`：`DocumentStore::apply` 经 WorkingSet 只落 touched 节点，编辑成本与改动范围成正比。
- `src/host/readiness.rs:1517-1522`：进程探针为启动一次性；`workbench_gui.rs:3016` 50ms 菜单轮询开销可忽略。

---

## language｜界面语言与本地化（JSON 语言包，不得写死代码）

### L-1 现状判定：结构性偏差 ⚠️

- **语言目录硬编码在 Rust 源码**：`src/app/locale/` 共 5 文件 1536 行（catalog.rs 614 / format.rs 284 /
  mod.rs 27 / vocabulary.rs 489 / zh_hans.rs 122）。简体中文 73 条写死为 const 数组
  （⚙ `zh_hans.rs:30` `pub const MESSAGES: [(&str,&str); 73]`）；诊断模板/进程文案同为 const 数组
  （`vocabulary.rs:79-111`）；英文"包"由命令注册表/菜单树**派生**而非独立文件（`vocabulary.rs:6-9,117`）。
- **语言包格式是自研行式文本，不是 JSON**：`Pack::encode/decode`（⚙ `catalog.rs:105-150`）写出
  `<tag>.lang` 文件（`run/locale.rs:103-104`），首行 `rustage-locale 1`——与"多语言以 JSON 语言包形式"的
  工作规则直接冲突。
- **GUI 完全未接 i18n**：`workbench_gui.rs` 中 grep `catalog|locale` 零命中。语言选择只影响 CLI
  诊断文案（`main.rs:33-38` 启动时 `catalog::select→install`；`commands/command.rs:517,694` 拒绝消息），
  不影响界面——原则 #67"语言切换影响界面文案"未达成。
- 边界澄清：`src/app/language/` 是代码语言服务（LSP），与 UI i18n 无混淆；它仅在 `language/target.rs:53`
  用上游 `rust_widgets::i18n::{I18nManager, TranslationFile}` 生成**应用**的语言资源（T24.04 边界），设计器
  自身 UI 未消费。

### L-2 硬编码文案盘点（GUI 层约 70–100 条）

菜单/工具栏文案是绕过已编目命令注册表的重复一套（`app/commands/menu.rs:69-90` 的 title 已入 vocabulary）。
代表性例子（⚙ 抽验）：

| 证据 | 文案 |
|---|---|
| `workbench_gui.rs:74-77` | 面板名 "Pages" "Events" "Data & State" "Debug" |
| ⚙ `workbench_gui.rs:1525-1552` | 菜单 4 组：("File",Quit)；View 9 项（Command Palette/Pages/Outline/Inspector/Messages/Problems/Events/Data & State/Debug）；Layout 2 项；Editor 2 项 |
| ⚙ `workbench_gui.rs:1563-1575` | 工具栏 11 个按钮（Commands/Outline/Inspector/Messages/Problems/Events/Data/Split/Next group/Save layout/Reset layout） |
| `workbench_gui.rs:186,277,284` | 状态栏 "no document"；"the editor surface did not accept the document" |
| `workbench_gui.rs:339,365,439-478,542,571` | 检查器/状态消息（"control {id} publishes no {event} signal"、"schema unavailable: {reason}"、"not applied: {problem}" 等 format! 串） |
| `shell/panels.rs:347-350` | PanelSpec 标题 "outline"/"Inspector"/"Messages"/"Problems" |
| `host/dialog.rs:86-89,221,306` | 选择器类别与结果消息 |
| `host/notify.rs:227-315` | 任务/进程描述消息 |
| `workbench_palette.rs:119,127-139` | 命令面板帮助行 "No command matches this query." 等 |

### L-3 上游 i18n 能力（已核实，宿主尚未消费）

- **格式 JSON**：`rw/language/{en,zh-cn,zh-tw}.json` 各 788 行；结构
  `{"language":"en","translations":{"key":{"message":"..."}}}`（en.json:1-6）；serde 类型
  `TranslationFile/Translation`（`rw/src/i18n/types.rs:11-45`，含 context 与 plural）。
- **热加载**：`rw/src/i18n/mod.rs:4` 自述 "internationalization support with hot reload"；
  `watcher.rs:83,106,407,444,482,501` 提供 `I18nFileWatcher`/`watch_directory`/`pump_hot_reload`（帧循环每帧泵）/
  `enable_disable_global_hot_reload`；环境变量 `RUST_WIDGETS_I18N_DIR`（`watcher.rs:424`）。
- **加载/查询**：`manager.rs:244` `load_translations`（serde_json，文件名词干==声明 language，:256-274）、
  `:286` `inject_translations`、`:290` `set_language`、`:313` `translate`、`:330` `translate_with_context`、
  `:298` `audit_keys`；缺 key 回退内嵌 en（:355-360）；初始化 `global.rs:33,74` + `init_with_options`（`options.rs:13-20`）。

### L-4 迁移缺口（宿主改造面，全部为记录）

1. 词条外置：`vocabulary.rs:79-111` 与 `zh_hans.rs:30-122` 转 JSON 语言包；"英文从注册表派生"的设计
   （`vocabulary.rs:6-9`）需改为构建期导出 JSON，否则违反"不得写死在代码里"。
2. 包格式替换：自研 `Pack::encode/decode`（`catalog.rs:105-150`）→ 上游 `TranslationFile` JSON；
   `.lang` 导出（`run/locale.rs:103-104`）同步退役。
3. GUI 接线（最大改造面，约 60+ 条）：`workbench_gui.rs`（菜单/工具栏/面板名/状态栏/消息/检查器）、
   `shell/panels.rs:347-350`、`workbench_palette.rs:119-139` 全部走 `translate`。
4. host 层文案：`host/dialog.rs:86-89,221,306`、`host/notify.rs:227-315`。
5. 热加载泵：GUI 帧循环调用 `pump_hot_reload`（`rw/src/i18n/watcher.rs:501`）并按 ReloadEvent 重刷控件文本
   （现有每帧 set 文本点：`workbench_gui.rs:289,422`）。
6. 语言选择贯通：`main.rs:33-38` 的选择结果传入 GUI 驱动 `set_language`+重渲染（现状只影响 CLI）。
7. tag 归一：宿主 `zh-Hans`（`host/locale.rs:64`）vs 上游文件名词干 `zh-cn`（`manager.rs:256-274`）需统一约定。

### L-5 上游 i18n 反馈候选（已并入 feedback-20261030.md）

- 上游 i18n 缺 `{name}` 占位符替换（宿主 `catalog.rs:295-310` 的 `Catalog::fill` 支持模板填充；
  上游 `src/i18n/` grep placeholder/interpolat 无命中，仅 context/plural）。
- 上游缺"跟随系统语言"探测（`InitOptions.language` 需调用方给定；宿主 `host/locale.rs:126-133` 已有实现）。
- 上游缺"加载期按 key 集合双向完整性校验"（`audit_keys` 仅事后审计；宿主 `zh_hans.rs:11-19` 以测试强制，弱需求）。

---

## 覆盖范围与未覆盖声明（原则 #25）

- **已覆盖**：roadmap.md §2–§4、F00–F28、§34/§35/§37 共 123 个 T 任务逐项对照；`src/` 全部 351 文件的
  控件引用清单、locale/language 模块全文、unsafe 全量（15 处）、`#[allow(dead_code)]` 全量（4 处带注释）、
  `todo!` 全量（0 处）；热路径 15 文件 28,666 行的效率扫描。
- **抽样覆盖**：运行路径 unwrap/expect（177 处抽约 30）；SPDX 头（抽 20 文件 + 全量计数）；GUI 硬编码文案（代表 20 组）。
- **未覆盖**：库内部（上游 `ViewEngine::update` 的成本）、`src/app/canvas/{design,drag,objects,structure,viewport}.rs`
  与 `src/project/document/{history,closing,conflict,recovery,buffer}.rs` 的逐行效率审计、`format.rs` encode_* 函数群逐行、
  实际运行时验证（本轮未运行任何构建/测试/程序）。
- 所有结论为静态阅读所得；标注 ⚙ 的条目已由主审计对源码原文二次核实。
