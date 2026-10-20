# blue13｜MS12 开发闭环联合验收

> 依据：[roadmap.md](./roadmap.md) §5 里程碑表及对应 F 域任务表。本文编号 13 对应 MS12；路线图是范围与契约基准，如有差异以路线图及批准的变更记录为准。

## 阶段目标与范围

- **阶段目标：** 开发闭环联合验收。
- **进入条件：** MS08–MS11 均已完成；仅在各组成能力真实接通后开始联合签收。
- **交付集合：** 跨域集成门禁，无新增实现任务。
- **退出门禁：** 设计到基础调试再到本地产物交付可复现，无 P0 缺口。
- **演示脚本：** 按 §34 逐条跑 MS12 适用的 J01–J09、J17–J19；覆盖正常、取消/错误/恢复和键盘入口。MS12 是集成门禁，不新造“写完即完成”的任务。
- **范围说明：** 核对 F09–F18、T19.01–T19.08、F26/F27 适用全链路；组件/界面/预览/生成/build/run/debug 源映射保持同一版本和事实来源。

## 执行约定

1. 按任务表中明确列出的前置关系推进；无依赖任务可并行，不把本文件排列顺序当作额外依赖。前置未满足时标记阻塞，不以占位实现代替。
2. 先核对公共契约、支持能力、权限与版本，再实现任务交付物；复用路线图已定义的事实来源和服务，缺少上游能力时记录缺口，不自行复制库实现或创造不兼容语义。
3. 各任务按适用范围验证正常、非法输入、取消、拒绝/冲突、服务故障和恢复；结果应可观察、可重现，失败不能显示为成功。
4. 逐任务记录责任人、状态、实现修订、测试命令/环境、自动/人工结果、目标/设备、产物、未覆盖范围与验收日期。未执行、缺设备或仅模拟的项目不得记为通过。
5. 状态使用“待开始 / 进行中 / 阻塞 / 待验收 / 完成”；实现完成不等于验收完成。

## 联合验收步骤

1. 冻结参与验收的工程修订、profile/Kit、工具链、平台和测试数据；确认 MS08–MS11 退出证据有效且版本一致。
2. 按指定旅程从用户入口执行设计态修改、文档事务、命令入口、预览、生成、检查/构建/运行及基础调试映射。
3. 对每条链执行成功及适用的取消、错误、权限拒绝、冲突和恢复路径；核对源版本、NodeId、产物、诊断与进程归属一致。
4. 使用受支持组合的真实宿主/目标产物验收；模拟 UI、截图或单独库侧测试不能替代端到端证明。
5. 保存旅程脚本、工程修订、命令/环境、日志、产物、人工步骤、问题和未覆盖范围；清除所有 P0 后签收。

## 联合验收规范

- 阶段门禁：设计到基础调试再到本地产物交付可复现，无 P0 缺口。
- MS12 是独立集成门禁，不添加“实现任务”；适用旅程未复现、存在 P0、版本证据不匹配或失败被伪装成成功时不得签收。

## 阶段完成签收

- [ ] 入口条件有证据，所有范围内任务前置已满足。
- [ ] 联合验收所有适用旅程均已复现；未关闭缺陷和未覆盖组合已明确列出。
- [ ] 退出演示、负向/取消/恢复路径及适用的性能、平台、目标、安全门禁均已通过。
- [ ] 对应用户旅程/命令/菜单/面板/支持矩阵已更新，后续依赖已确认；无未经批准的 FUTURE 依赖。
- **验收结论：** 未签收（2026-10-20 第三次复核，第四至第七次补充后仍不签收）。实现链与
  负向/恢复路径已复现，命令面板、属性编辑、对象树与画布、调试面板均已取得真实窗口证据；
  但固定依赖 2.8.11 的 embedded profile 不能编译，事件/数据运行检查与调试面板的双向源导航
  仍未接通，因此 J02/J03/J09 的对应 GUI 部分仍不完整。
- **验收人 / 日期 / 工程修订：** 待验收人填写；本轮证据见
  [`docs/log/log-20261020-3.md`](../log/log-20261020-3.md)。

### 2026-10-20 第三次复核：实际复现了什么

> 环境：Linux x86_64（Ubuntu 24.04 内核，X11 `DISPLAY=:0`）、Rust 1.98.1、
> `rust_widgets` 2.8.11 @ `cee6b22367244527aa407bb9ae67305751f61c26`、GTK 3.24.41、
> `/usr/bin/gdb` 15.1（`--interpreter=dap`）、fcitx5。
> 脚本：[`tools/ms12-journeys/run.py`](../../tools/ms12-journeys/run.py)；报告：
> `target/ms12-journeys/report.json`（默认被 `.gitignore` 忽略，命令可重跑）。

脚本把每一步标成三种证据，避免把“模型通过”写成“界面完成”：

- **real**：读写真实文件或启动真实进程（cargo、gdb、业务 worker、生成程序）；
- **model**：通过真实 CLI 驱动类型化模型，但没有窗口或设备参与；
- **blocked**：本构建或固定依赖没有所需能力；原因逐条写入报告。

| 旅程 | 覆盖方式 | 结果 | 关键证据 |
|---|---|---|---|
| J01 新用户首个应用 | 新建→诊断→插入/属性→保存→生成→独立构建运行 | 全部 real，通过 | `--new-project`、`--validate --diagnose`、`--edit`、`--save`、`--generate`；`tools/generated-project/run.py` 得到 `generated-ui-ok` |
| J02 多页面应用 | 页面/入口/导航/返回/关闭 | 模型通过；窗口内页面编辑 blocked | `--routes` 的 2 页、1 边、go/back/forward；缺页目标得到 `no page "detail" is declared`；窗口已有对象树/画布，但没有页面与路由编辑入口 |
| J03 键盘完整工作流 | 命令注册、palette、键位、菜单审计 | 模型通过；窗口命令面板与对象树/画布 real；Enter/Esc 与键盘遍历仍缺 | `--keys --press Ctrl+P` → `view.palette`；`--menu --audit` 每个已实现叶项绑定一个命令；真实窗口：面板过滤/Run/Close/焦点恢复，Outline 行与画布选择一致 |
| J04 复杂布局与主题 | 布局计算、切换、约束、令牌、设备帧；窗口属性编辑、对象树与画布多选 | 属性编辑、树/画布点击与框选 real；主题多选审阅 blocked | preview 与 generation 共用一次计算；`--tokens` 保留引用、只在 `expand` 内联；真实窗口：非法标题被拒且修订 0 不变，合法值推进到修订 1，框选得到 `4 node(s): 0 window, 1 Demo, 2 button, 3 label` |
| J05 组件复用 | 定义/实例/展开/提取/覆盖/detach/reset/包路径 | 模型通过 | `expanded 2 node(s)`；包入口 `../escape.json` 被拒为 `escapes the project root` |
| J06 源码往返 | 生成→映射→手写区→反向解析→合并→diff | 全部 real，通过 | `node 1 -> src/generated/ui.rs:577..625`；手写行以 `kept (unrecognised)` 保留；一行编辑得到 `diff: 1 changed row(s)` |
| J07 带状态逻辑修改 | 热更新分类、双进程快照/导入、迟到帧隔离 | real 通过；GUI 焦点保留 blocked | `snapshot generation 1: running`、`restored generation 2: running`、`fault=the frame is from generation 1, not 2`；`tests/logic_process.rs` 通过 |
| J08 多目标交付 | 目标配置、能力快照、受限容量、矩阵、队列、mini 产物 | 模型与 mini real 通过；embedded 与真机安装 blocked | mini 消费者 crate 得到 `generated-mini-ok`；embedded 见 P0-1 |
| J09 调试业务事件 | 探针、断点、变量、求值、步进、attach/detach；窗口内调试面板 | 全部 real，通过；双向源导航 blocked | `variable n: i32 = 41`、`evaluate i32 41`、`stopped: step`；attach 后外部 `sleep` 仍 `alive`；面板真实 gdb 会话：`bound at 3`、`thread 1 app`、`n: i32 = 41`、Step/Run/Stop |
| J17 完整桌面应用外壳 | 动作/菜单/工具栏/状态/对话框/回归 | 模型通过；目标应用窗口 blocked | 菜单与工具栏读同一 action；`app-regression` 的 runtime/static 进入同一入口 |
| J18 数据密集页面 | 数据/绑定/表/集合/单元格/大集合 | 模型通过 | 样例与运行时数据分开；不兼容绑定 `REFUSED`，`f:total→1.text` 用 `text` 转换才 `accepted`；100 行只显示 0..20 |
| J19 多根/多包工程 | 新建 workspace、成员、活动成员、同名文件、离线解析 | real 通过 | `--workspace --add --apply --trust` 写入成员；`member:src/main.rs` 按成员限定；`--dependencies --allow-lockfile-change` 离线解析 |

汇总：74 步，39 模型级通过、27 真实级通过、8 阻塞、0 失败。

### 本轮发现并修复的集成缺陷

1. **页面文档到生成器没有消费者（P0）。** `--generate document:<page>` 只接受库的 widget JSON，
   而 `app::generate::document::to_library_document` 虽有实现却标着 `#![allow(dead_code)]`，
   没有任何调用点。J01“设计→生成”因此断在 seam 上。修复：`read_document` 识别页面文档并调用
   该唯一翻译；原始 widget JSON 路径保持不变。新增单元测试覆盖翻译。
2. **新工程的根节点不是生成器可消费的控件（P0）。** 模板把页面根建成 `page` 类型，生成器只认识
   `window`，所以新建工程即使插入控件也无法生成。修复：新页面根使用 `window`（决策 Q5 把页面/
   窗口作为同一入口单位）；旧工程若根仍是 `page`，生成会以“no control by that name”明确拒绝，
   不静默改写文档。
3. **支持矩阵负向测试的固定行随证据更新而失效（P1）。** `check-support-matrix.test.mjs` 仍去替换
   一行 `unverified` 的 macos 行，而该行早已是 `build-verified`；测试因此“无失败可报”。改为替换
   linux 的首个 `build-verified` 格，负向用例重新有效。

### 未关闭缺陷（P0/P1）

- **P0-1：2.8.11 的 embedded profile 不能编译。** `WidgetKind::{TabWidget,Splitter,Grid,Dialog}`
  由 `widgets_unstripped` 门控，而四个新构造器由 `not(alloc_frugal)` 门控；`embedded` 两个都为假，
  得到 E0599。`tools/generated-project/run.py --profile embedded` 因此失败。矩阵 embedded 格标
  `unsupported`，缺陷记入 [`feedback-20261020.md` §0.1](../rw-feedback/feedback-20261020.md)。
- **P0-2：工作台窗口的 GUI 面部分接通。** 命令面板、属性编辑、对象树与画布已接入真实窗口：
  命令面板（T05.04/T05.05）有工具栏入口、真实输入过滤、行选择、Run/Close 与焦点恢复；
  属性编辑（T11.01）有真实 `LineEdit`、Apply/Clear、非法值保留、有效值推进修订号；
  对象树（T09.02–T09.04/T09.08）与画布共用同一选择集合，支持点击最上层、框选、隐藏/锁定
  拒绝，并驱动 Inspector 重命名节点；调试面板（T19.03–T19.05）驱动真实 gdb DAP 会话，
  显示适配器验证的断点绑定、线程/栈/局部变量，并提供 Step/Run/Stop。
  仍未接通：属性多选交集（T11.02）、调试面板的双向源导航（T19.06）与事件/数据运行检查
  （T13.03/T14.05）；J02 的页面/路由编辑仍缺，J04 的主题多选审阅仍只有模型。
  **归属：** 不新增任务；这些能力已在路线图中由 T05.04（命令面板，MS03，本轮完成）、
  T09.02–T09.08（画布/对象树，MS05）、T11.01–T11.02（属性编辑，MS05，T11.01 本轮完成）、
  T19.03–T19.06（调试面板，MS11）与 T13.03/T14.05（事件/数据与运行检查，MS06）覆盖，
  只是各阶段的 GUI 部分被当作边界延期。后续顺序、判据与门禁见
  [`issue-20261020.md`](../issues/issue-20261020.md)。在对应任务恢复为“进行中”并取得
  窗口证据前，MS12 不得签收。
- **P0-3：GTK 键盘与指针键码/双击未规范化。** 2.8.11 的 GTK 路径把 GDK `Return`（0xFF0D）
  与 `Escape`（0xFF1B）原样送入控件层，而 `LineEdit` 匹配 13/27，所以查询框的 Enter/Escape
  不触发；指针路径也不合成 `MouseDoubleClick`，所以 `ListBox` 的 `item_activated` 不因双击触发。
  命令面板改用 Run/Close，属性编辑改用可见值字段；探针把 Enter/Escape 与双击标 `blk`。
  缺陷见 [`feedback-20261020.md` §0.2/§0.3](../rw-feedback/feedback-20261020.md)。
- **P1-1：macOS/Windows 未在 2.8.11 上复跑。** 支持矩阵已退回 `unverified`，不拿 2.8.10 证据冒充。
- **P1-2：生成程序不是目标应用外壳。** `--app-*` 模型完整，但生成物只有 `build_ui()`，没有原生菜单、
  对话框或状态栏；J17 的“构建运行”因此只到独立程序，不到目标应用窗口。

### 2026-10-20 第四次补充：命令面板与焦点路由（第 1 步）

- **实现：** `src/app/run/workbench_palette.rs` 持有查询、选中行与打开前的焦点；
  `workbench_gui.rs` 用真实 `PanelHandle` + `LineEdit` + `ListBox` 镜像它；
  搜索、最近使用、禁用原因与参数帮助来自 `app::commands::palette`，没有第二份命令清单。
- **焦点：** 打开时保存 `focused_widget()` 并 `focus_widget(query)`；关闭时恢复原控件，
  失败则 `clear_focus()`。列表重建被排到下一轮主循环，避免控件信号回调重入 `Gui` 互斥量。
- **真实窗口证据：** `tools/gui-smoke/palette.py` 在自建 Xephyr 上点击工具栏、真实键入
  `welcome`、得到 `palette query "welcome": 1 result(s)`，点击 Run 得到 `palette ran view.welcome`，
  关闭后焦点回到打开前的控件；行点击后 Run 执行所选命令。
- **阻塞：** Enter/Escape 因 P0-3（GTK keyval）不触发；探针把这两项标 `blk`，没有用
  `dispatch_event` 或手工发信号冒充按键。
- **仍未做：** 参数值输入（当前只显示参数帮助，不提供逐参数编辑）、方向键在查询框内的
  列表导航（需 P0-3 修复）、最近使用的持久化。

### 2026-10-20 第五次补充：属性编辑（第 2 步）

- **实现：** `src/app/run/workbench_property.rs` 用现有 `app::style::property_editor` 的
  `Editor`/`Schema` 持有一个节点的 `title`（`Text{max_len:40}`）与修订号；Inspector 面板内
  新增真实 `LineEdit`、Apply、Clear 与错误标签，由面板自己的 `BoxLayout` 排布。
- **判据：** 非法值（超过 40 字符）保留在字段里，错误行显示原因，有效值与修订号不变；
  Clear 只清空字段、不改有效值；合法值推进修订号一次并刷新 Inspector 行。
- **真实窗口证据：** `tools/gui-smoke/property.py` 在 Xephyr 上真实输入 45 个字符、点击 Apply
  得到 `revision 0 unchanged`；点击 Clear 后输入 `test` 并 Apply，得到
  `inspector applied title = "test"; revision 1` 与 `inspector rows: title: test`。
- **未做：** 双击/Enter 进入编辑受 P0-3 阻塞（可见字段可直接编辑）；多选交集与
  “重置恢复来源”（T11.02）要等第 3 步的选择集合；参数化属性类型只在模型里覆盖。

### 2026-10-20 第六次补充：对象树与画布（第 3 步）

- **实现：** 新增 `src/app/run/workbench_scene.rs`。`CanvasState` 同时持有结构树
  （`app::canvas::structure::Tree`）、场景实例（`app::canvas::scene::Scene`）与唯一选择集合；
  Outline 的每一行、画布的命中测试和 Inspector 都读这一个状态，不存在第二份坐标或选择。
- **画布：** 使用库的真实 `Canvas` 控件（经 control backend 的 `create_canvas`；
  `create_widget_of_kind` 在原生桌面后端返回 0，见上游反馈 §0.4）。节点由画布自身绘制，
  扁平配色（浅灰底、白卡片、蓝色选中边框）；不用子控件覆盖，因此框选可以跨过节点。
- **判据：** 重叠时点击选中最上层；树行与画布点击得到同一节点；隐藏节点拒绝并说明
  `hidden`，锁定节点拒绝并说明 `locked`；拖拽框选只包含可选交集；Inspector 随选择重定向，
  Apply 后重命名树节点并刷新两处视图。
- **真实窗口证据：** `tools/gui-smoke/scene.py` 9 项通过，日志包含
  `refused tree selection of node 4: node 4 is hidden`、
  `refused tree selection of node 5: node 5 is locked`、
  `selected node 3 (label) from canvas` 与
  `marquee 20,20 330x230: 4 node(s): 0 window, 1 Demo, 2 button, 3 label`；截图
  `target/scene-after.png` 显示 Outline、画布选中框与 Inspector 同步。
- **未做：** 键盘焦点遍历、删除/重排/重父的窗口动作、主题多选审阅；原始窗口 resize 后
  画布命令要等下一次选择或指针事件才按新矩形重画（split/面板切换会主动重画）。

### 2026-10-20 第七次补充：调试面板（第 4 步）

- **实现：** 新增 `src/app/run/workbench_debug.rs`。面板按钮把命令发给后台 worker；worker 用
  与 `--debug` 相同的 `Client`/`Book` 启动真实 gdb DAP：`rustc -g` 编译 debuggee，
  `launch` staged at main，`setBreakpoints`，`configurationDone`，`continue`，然后读取
  线程/栈/作用域/变量。窗口只轮询快照，不阻塞协议。
- **判据：** 断点只在适配器 `verified` 后显示 `bound`；线程/栈/局部变量来自真实请求；
  Step/Run/Stop 真实驱动程序；面板显示 `main.rs:3:0` 并明确 `unmapped`，不猜节点。
- **真实窗口证据：** `tools/gui-smoke/debug.py` 14 项通过；日志含
  `debug: stopped: breakpoint at line 3 (thread 1)`、`breakpoint 1 at main.rs:3: bound at 3`、
  `thread 1 app`、`frame 0 main::main at main.rs:3:0`、`variable n: i32 = 41`、
  `debug: exited with code 0`、`debug: session ended`。
- **性能修复：** 画布重画按最新快照合并（8ms、一个在途帧）；debuggee 编译带命令戳缓存；
  探针用 C 级 `atexit` 清理并用无锁显示号启动 Xephyr，异常退出不再留下窗口/服务器。
- **未做：** T19.06 双向源导航需要工程 `SourceMap`；断点编辑、线程/帧切换、变量展开、求值/
  监视区；事件/数据运行检查（第 5 步）。

### 阶段门禁结论

设计→生成→独立构建运行→真实 gdb 调试这条主链可复现，命令面板、属性编辑、对象树与画布、
调试面板也取得了真实窗口证据；但 embedded 产物、P0-3 的键盘/双击键码、调试双向源导航与
事件/数据运行检查仍未关闭，按本文“存在 P0 不得签收”执行：**MS12 未签收**。
