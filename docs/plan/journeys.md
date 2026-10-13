# 旅程走查与模式原型：§34 逐步骤入口与归属

> 数据源：[`journeys.json`](journeys.json)。本文件的表格由
> [`tools/journeys/check-journeys.mjs`](../../tools/journeys/check-journeys.mjs) 逐行对照该文件
> 校验；同一脚本还解析[路线图](roadmap.md) §34 的旅程表并比对旅程编号、名称、必需证据、首验/收敛
> 与**逐步的操作链**，解析 `resources/host/icons/manifest.json` 核对每个入口是否落在已声明的
> 分组通配或面板之下，并在 `roadmap.md` 中核对每个归属任务是否真实存在。
> 依据：[项目原则](principle.md) #25/#26/#38、[路线图](roadmap.md) §2.1/§3/§34、[白皮书](whitepaper.md) §17.1/§17.2、
> [决策登记](decisions.md)、[公共契约](contracts.md)。
> 状态：**7 个模式原型与 26 条旅程的 171 个步骤都已给出入口（原型决定）与归属任务；
> 没有任何入口已实现**（`implementation: none`）——本项交付的是设计与追溯，不是可运行的界面。

## 这份走查回答什么

T00.06 的最低验收是「完成 §34 旅程原型走查，缺入口有任务归属」。它有两个可检查的含义：

1. **每个旅程步骤都有入口。** 入口是本任务给出的具体菜单叶项（如 `design.node.insert`）或面板
   （如 `panel.properties`）。叶项必须落在 `resources/host/icons/manifest.json` 已声明的分组通配
   之下，或直接是已声明的面板 id——也就是说，走查可以细化菜单，但不能凭空新增一整条菜单分支而
   不经过 T05.02 的菜单树。
2. **每个步骤都有归属任务。** 归属是路线图里真实存在的任务 id（或 `FUTURE`，表示该步骤属于不进入
   正式版的未来扩展）。没有入口的步骤必须写明理由。

「有入口」是**设计结论**，不是实现状态：归属任务列给出谁来实现它。这条边界写在状态行里，也由检查
脚本强制（`implementation` 必须是 `none`，直到有任务真正把某个入口接上）。

## 模式原型

路线图 §2.1 定义的模式是 7 个：**欢迎、工程、设计、代码、运行与调试、测试、发布**。T00.06 的任务
描述里写的是其中五个（欢迎/设计/代码/调试/发布）；本走查按路线图的完整列表执行，并把它记在这里，
以免读者以为少了两个模式。

每个模式给出默认面板与代表入口。**面板的可发现性是可检查的**：旅程步骤用到的每个面板都必须至少出现
在一个模式的默认面板里，否则用户在设计器里找不到它。命令组到模式的完整映射由 T05.02 的菜单树实现，
本项不声称已定。

| 模式 | 名称 | 作用 | 默认面板 | 代表入口 |
| --- | --- | --- | --- | --- |
| welcome | 欢迎 | 进入工程：新建/打开/最近/恢复 | `panel.help`, `panel.notifications` | `view.mode.welcome`, `file.new.project`, `file.open.project`, `file.open.recent`, `help.about` |
| project | 工程 | 工程树的组织与工程级配置 | `panel.project`, `panel.assets`, `panel.pages`, `panel.targets`, `panel.dependencies`, `panel.environments`, `panel.problems` | `project.structure.reveal`, `project.config.kit`, `project.workspace.add`, `project.environment.toolchain` |
| design | 设计 | 画布、对象树、属性、样式与组件 | `panel.objects`, `panel.widgets`, `panel.properties`, `panel.layout`, `panel.styles`, `panel.events`, `panel.data`, `panel.components`, `panel.preview`, `panel.problems` | `design.node.insert`, `design.tool.select`, `design.property.value`, `design.layout.nest`, `design.component.extract`, `preview.session.start` |
| code | 代码 | 设计文档的 JSON/CSS/生成 Rust 视图与文本编辑 | `panel.outline`, `panel.search`, `panel.problems`, `panel.tasks` | `view.mode.code`, `code.edit.page`, `code.sync.diff`, `edit.find.project`, `nav.open.symbol`, `edit.find.cancel` |
| run-debug | 运行与调试 | 构建、运行、预览热更新与调试会话 | `panel.build`, `panel.run`, `panel.terminal`, `panel.tasks`, `panel.hot`, `panel.breakpoints`, `panel.stack`, `panel.variables`, `panel.debug`, `panel.problems` | `build.exec.check`, `build.exec.build`, `run.app.start`, `run.app.debug`, `debug.breakpoint.toggle`, `debug.exec.step`, `preview.hot.update`, `preview.hot.migrate` |
| test | 测试 | 设计检查、状态/视觉/性能测试与证据 | `panel.tests`, `panel.problems`, `panel.tasks`, `panel.profiler`, `panel.memory`, `panel.inspection` | `test.analyze.design`, `test.run.state`, `test.report.visual`, `test.profile.performance`, `test.report.export` |
| release | 发布 | 目标矩阵、产物导出、更新与长期维护 | `panel.targets`, `panel.build`, `panel.dependencies`, `panel.problems`, `panel.vcs` | `preview.matrix.generate`, `preview.matrix.build`, `build.output.export`, `build.output.install`, `tools.settings.update`, `tools.settings.uninstall` |

## 旅程与四个方面的定位

| 旅程 | 名称 | 范围 | 必需证据 | 首验/收敛 | 正常流程 | 取消/错误 | 恢复 | 键盘 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| J01 | 新用户首个应用 | release | 独立生成工程与真实进程，缺工具链可恢复 | MS07/MS18 | 1 欢迎 | 8 构建 | 3 工具链诊断 | 4 插入 |
| J02 | 多页面应用 | release | 页面/状态/焦点一致，删路由引用可诊断 | MS06/MS12 | 1 新页面 | 5 窗口关闭 | 2 设入口 | 1 新页面 |
| J03 | 键盘完整工作流 | release | 不用鼠标可完成，焦点不会被任务抢走 | MS07/MS17 | 1 命令面板 | 7 构建 | 7 构建 | 1 命令面板 |
| J04 | 复杂布局与主题 | release | Layout/CSS 语义一致，约束失败可定位 | MS07/MS12 | 1 嵌套布局 | 6 检查 | 6 检查 | 2 多选 |
| J05 | 组件复用 | release | 多实例与静态输出一致，循环拒绝 | MS08/MS12 | 1 提取 | 7 撤销 | 7 撤销 | 1 提取 |
| J06 | 源码往返 | release | 手写保留、位置准确、取消不丢两侧内容 | MS07/MS12 | 1 UI 改动 | 6 冲突 | 6 冲突 | 3 JSON/CSS 改动 |
| J07 | 带状态逻辑修改 | release | 版本组合正确；编译失败旧版真实继续 | MS09/MS12 | 1 输入/滚动 | 4 迁移 | 5 切换 | 1 输入/滚动 |
| J08 | 多目标交付 | release | profile/triple 分离，受限产物无开发依赖；不要求远程部署 | MS10/MS18 | 1 切目标 | 5 导出/本地设备安装 | 7 设备验收 | 4 本地矩阵构建 |
| J09 | 调试业务事件 | release | 真实停止/变量/源版本，栈不假保留 | MS11/MS12 | 1 绑定 | 7 重新附加 | 7 重新附加 | 5 步进 |
| J10 | 版本与多人异步协作 | release | 无需实时协作，仓库身份明确，恢复冲突不丢脏内容 | MS13/MS18 | 1 diff | 7 合并 | 7 合并 | 2 暂存 |
| J11 | 自动质量检查 | release | 测试/未运行/误差/环境真实分列 | MS14/MS18 | 1 设计检查 | 5 失败重跑 | 5 失败重跑 | 1 设计检查 |
| J12 | 可选 AI 变更 | release | 不外发未授权数据，人工仍完整 | MS16/MS18 | 1 授权上下文 | 5 部分接受 | 5 部分接受 | 1 授权上下文 |
| J13 | 大工程日常编辑 | release | 无逐帧全文工作，反馈/内存达基线 | MS04/MS18 | 1 百万行打开 | 4 取消 | 7 关闭恢复 | 5 符号 |
| J14 | 异常恢复 | release | 原内容/最后有效模型和来源可证明 | MS02/MS18 | 1 保存中断/磁盘满/外部冲突/服务崩溃 | 2 诊断 | 3 恢复 | 4 继续 |
| J15 | 扩展与自定义控件 | release | 无契约不启用，扩展失败不崩宿主 | MS15/MS18 | 1 注册 | 5 启停 | 6 升级失败恢复 | 2 schema |
| J16 | 安装与长期维护 | release | 不删工程，不宣传格式无损降级 | MS18/MS19 | 1 干净安装 | 4 更新中断 | 5 恢复 | 2 旧工程 |
| J17 | 完整桌面应用外壳 | release | 目标应用真实入口同源，关闭/取消/焦点正确 | MS08/MS12 | 1 动作 | 5 模态弹窗 | 6 构建运行 | 3 快捷键 |
| J18 | 数据密集页面 | release | 内容与事件真实，加载失败和大集合不冻结 | MS08/MS12 | 1 表列/树项 | 6 空/错误 | 6 空/错误 | 5 编辑校验 |
| J19 | 多根/多包工程 | release | 根/包/目标/信任身份正确，缺缓存不暗自联网 | MS07/MS18 | 1 添加成员 | 4 依赖变更审阅 | 5 离线构建 | 2 同名文件导航 |
| J20 | 任务与 CI 复现 | release | GUI/CLI 复用语义，取消不留服务；错误/未运行/陈旧结果区分 | MS14/MS18 | 1 前置任务 | 4 headless 检查/测试 | 4 headless 检查/测试 | 1 前置任务 |
| J21 | 内置性能定位（未来扩展） | future | 原始报告/修订/工具/开销可复核，不把缺数据说成优化成功 | FUTURE；不进入正式版验收 | 1 选进程 | 4 取消/符号化 | 4 取消/符号化 | 3 复现 |
| J22 | 远程/容器工作（未来） | future | 无上传/执行偷跑，未确认写入不报成功，不误杀或删外部资源 | FUTURE；不进入正式版验收 | 1 选环境 | 7 断线 | 8 重连 | 3 连接 |
| J23 | 双 AI 闭环 | release | 角色与候选版本明确；旧评审不授权新候选，目录不是权限隔离 | MS16/MS18 | 1 coder 工作区 | 7 人工接受 | 6 重审 | 1 coder 工作区 |
| J24 | 翻译与表单 | release | 字符/布局/键/焦点不漂移，过期校验不触发业务，无能力项明确诊断 | MS17/MS18 | 1 提取键 | 6 双模式运行 | 6 双模式运行 | 5 提交取消 |
| J25 | 配置与故障排查 | release | 不导入权限/机密；定位到真实服务，不删工程或强制注册账号 | MS17/MS18 | 1 导入快捷键/配置 profile | 3 启动失败 | 6 恢复 | 1 导入快捷键/配置 profile |
| J26 | 高级原生与复合调试（未来扩展） | future | 适配能力/符号/进程身份可证，附加外部进程不误停，不假保留调用栈 | FUTURE；不进入正式版验收 | 1 选包/运行配置 | 5 停止 | 5 停止 | 2 双进程调试 |
## 旅程逐步骤走查

171 个步骤逐条给出入口与归属。为保持可核对，本表由数据源生成后再人工复核；步骤文字与路线图 §34 的
操作链逐字一致（由检查脚本比对，改一边而漏另一边会失败）。

| 旅程 | 步骤 | 入口 | 归属任务 | 说明 |
| --- | --- | --- | --- | --- |
| J01 | 1. 欢迎 | `view.mode.welcome` | `T04.01` |  |
| J01 | 2. 新建 | `file.new.project` | `T02.02` |  |
| J01 | 3. 工具链诊断 | `project.environment.toolchain` | `T02.03` |  |
| J01 | 4. 插入 | `design.node.insert` | `T09.04` |  |
| J01 | 5. 属性 | `design.property.value` | `T11.01` |  |
| J01 | 6. 保存 | `file.save.page` | `T03.04` |  |
| J01 | 7. 生成 | `preview.session.start` | `T14.01` |  |
| J01 | 8. 构建 | `build.exec.check` | `T16.02` |  |
| J01 | 9. 运行 | `run.app.start` | `T16.03` |  |
| J02 | 1. 新页面 | `design.content.page` | `T13.05` |  |
| J02 | 2. 设入口 | `design.application.entry` | `T13.05` |  |
| J02 | 3. 参数导航 | `design.flow.navigate` | `T13.05` |  |
| J02 | 4. 返回 | `nav.open.previous` | `T13.05` |  |
| J02 | 5. 窗口关闭 | `window.close` | `T03.07` |  |
| J02 | 6. 运行 | `run.app.start` | `T16.03` |  |
| J03 | 1. 命令面板 | `nav.open.command` | `T05.04` |  |
| J03 | 2. 打开 | `file.open.project` | `T02.03` |  |
| J03 | 3. 插入 | `design.node.insert` | `T09.04` |  |
| J03 | 4. 树选择 | `design.inspect.select` | `T09.03` |  |
| J03 | 5. 属性 | `design.property.value` | `T11.01` |  |
| J03 | 6. 代码 | `view.mode.code` | `T06.01` |  |
| J03 | 7. 构建 | `build.exec.check` | `T16.02` |  |
| J03 | 8. 问题导航 | `nav.issue.next` | `T14.05` |  |
| J04 | 1. 嵌套布局 | `design.layout.nest` | `T10.02` |  |
| J04 | 2. 多选 | `design.inspect.multi` | `T09.03` |  |
| J04 | 3. 令牌 | `design.property.tokens` | `T11.04` |  |
| J04 | 4. 主题 | `design.property.theme` | `T11.05` |  |
| J04 | 5. 尺寸 | `view.display.device` | `T10.05` |  |
| J04 | 6. 检查 | `design.inspect.issues` | `T10.01` |  |
| J04 | 7. 生成 | `preview.session.start` | `T14.01` |  |
| J05 | 1. 提取 | `design.component.extract` | `T12.02` |  |
| J05 | 2. 实例 | `design.component.instance` | `T12.02` |  |
| J05 | 3. 覆盖 | `design.component.override` | `T12.03` |  |
| J05 | 4. 变体 | `design.component.variant` | `T12.03` |  |
| J05 | 5. 定义更新 | `design.component.definition` | `T12.02` |  |
| J05 | 6. diff | `design.component.diff` | `T12.02` |  |
| J05 | 7. 撤销 | `edit.history.undo` | `T03.03` |  |
| J05 | 8. 导出导入 | `file.transfer.component` | `T12.07` |  |
| J06 | 1. UI 改动 | `design.property.value` | `T11.01` |  |
| J06 | 2. 局部源码 diff | `code.sync.diff` | `T15.04` |  |
| J06 | 3. JSON/CSS 改动 | `code.edit.page` | `T06.01` |  |
| J06 | 4. 模型 | `code.sync.model` | `T15.04` |  |
| J06 | 5. 手写逻辑 | `code.edit.handwritten` | `T06.06` |  |
| J06 | 6. 冲突 | `code.sync.conflict` | `T20.07` |  |
| J07 | 1. 输入/滚动 | `preview.hot.update` | `T14.02` |  |
| J07 | 2. Rust 修改 | `code.edit.handwritten` | `T06.01` |  |
| J07 | 3. 候选 | `preview.hot.candidate` | `T14.01` |  |
| J07 | 4. 迁移 | `preview.hot.migrate` | `T14.03` |  |
| J07 | 5. 切换 | `preview.hot.switch` | `T18.05` |  |
| J07 | 6. 新事件 | `debug.inspect.events` | `T14.05` |  |
| J08 | 1. 切目标 | `build.config.target` | `T02.06` |  |
| J08 | 2. 能力预检 | `design.inspect.capability` | `T21.02` |  |
| J08 | 3. 生成 | `preview.matrix.generate` | `T15.02` |  |
| J08 | 4. 本地矩阵构建 | `preview.matrix.build` | `T16.08` |  |
| J08 | 5. 导出/本地设备安装 | `build.output.export` | `T25.02` |  |
| J08 | 6. 日志 | `build.output.log` | `T16.02` |  |
| J08 | 7. 设备验收 | `test.run.device` | `T25.01` |  |
| J09 | 1. 绑定 | `debug.breakpoint.bind` | `T13.03` |  |
| J09 | 2. 启动调试 | `run.app.debug` | `T19.02` |  |
| J09 | 3. 断点 | `debug.breakpoint.toggle` | `T19.03` |  |
| J09 | 4. 变量 | `debug.inspect.variables` | `T19.05` |  |
| J09 | 5. 步进 | `debug.exec.step` | `T19.04` |  |
| J09 | 6. 热更新暂停 | `preview.hot.pause` | `T18.05` |  |
| J09 | 7. 重新附加 | `debug.session.attach` | `T19.07` |  |
| J10 | 1. diff | `vcs.change.review` | `T20.02` |  |
| J10 | 2. 暂存 | `vcs.change.stage` | `T20.04` |  |
| J10 | 3. 提交 | `vcs.commit.create` | `T20.04` |  |
| J10 | 4. worktree/stash | `vcs.integrate.worktree` | `T20.05` |  |
| J10 | 5. 外部变更 | `vcs.change.external` | `T20.08` |  |
| J10 | 6. 分支切换 | `vcs.integrate.branch` | `T20.05` |  |
| J10 | 7. 合并 | `vcs.integrate.merge` | `T20.07` |  |
| J10 | 8. 重生成 | `preview.matrix.generate` | `T15.07` |  |
| J11 | 1. 设计检查 | `test.analyze.design` | `T21.02` |  |
| J11 | 2. 状态测试 | `test.run.state` | `T21.05` |  |
| J11 | 3. 视觉 | `test.report.visual` | `T21.04` |  |
| J11 | 4. 性能 | `test.profile.performance` | `T21.06` |  |
| J11 | 5. 失败重跑 | `test.run.retry` | `T21.05` |  |
| J11 | 6. 证据导出 | `test.report.export` | `T21.07` |  |
| J12 | 1. 授权上下文 | `tools.ai.context` | `T23.03` |  |
| J12 | 2. 提议 | `tools.ai.propose` | `T23.04` |  |
| J12 | 3. 候选预览 | `preview.hot.candidate` | `T23.04` |  |
| J12 | 4. 验证 | `test.analyze.design` | `T23.05` |  |
| J12 | 5. 部分接受 | `vcs.change.review` | `T23.06` |  |
| J12 | 6. 保存 | `file.save.page` | `T03.04` |  |
| J13 | 1. 百万行打开 | `file.open.large` | `T06.07` |  |
| J13 | 2. 分栏 | `view.layout.split` | `T04.04` |  |
| J13 | 3. 搜索 | `edit.find.project` | `T08.02` |  |
| J13 | 4. 取消 | `edit.find.cancel` | `T08.08` |  |
| J13 | 5. 符号 | `nav.open.symbol` | `T07.04` |  |
| J13 | 6. 编辑 | `code.edit.page` | `T06.01` |  |
| J13 | 7. 关闭恢复 | `file.close.recover` | `T03.06` |  |
| J14 | 1. 保存中断/磁盘满/外部冲突/服务崩溃 | `file.save.page` | `T03.04` |  |
| J14 | 2. 诊断 | `panel.problems` | `T01.06` |  |
| J14 | 3. 恢复 | `file.open.recover` | `T03.06` |  |
| J14 | 4. 继续 | `nav.issue.next` | `T14.05` |  |
| J15 | 1. 注册 | `tools.extension.register` | `T22.02` |  |
| J15 | 2. schema | `tools.extension.schema` | `T22.02` |  |
| J15 | 3. 预览 | `preview.session.start` | `T22.03` |  |
| J15 | 4. 目标生成 | `preview.matrix.generate` | `T22.03` |  |
| J15 | 5. 启停 | `tools.extension.toggle` | `T22.05` |  |
| J15 | 6. 升级失败恢复 | `tools.extension.recover` | `T22.06` |  |
| J16 | 1. 干净安装 | `build.output.install` | `T25.02` |  |
| J16 | 2. 旧工程 | `project.maintain.open-old` | `T02.07` |  |
| J16 | 3. 迁移 | `project.maintain.migrate` | `T02.07` |  |
| J16 | 4. 更新中断 | `tools.settings.update` | `T25.03` |  |
| J16 | 5. 恢复 | `tools.settings.recover` | `T25.03` |  |
| J16 | 6. 卸载 | `tools.settings.uninstall` | `T25.06` |  |
| J17 | 1. 动作 | `design.application.action` | `T26.02` |  |
| J17 | 2. 菜单/工具栏复用 | `design.application.menu` | `T26.03` |  |
| J17 | 3. 快捷键 | `design.application.shortcut` | `T26.04` |  |
| J17 | 4. 状态栏 | `design.application.status` | `T26.05` |  |
| J17 | 5. 模态弹窗 | `window.dialog` | `T26.06` |  |
| J17 | 6. 构建运行 | `build.exec.build` | `T26.08` |  |
| J18 | 1. 表列/树项 | `design.content.table` | `T27.02` |  |
| J18 | 2. 样例 | `design.content.sample` | `T27.01` |  |
| J18 | 3. 数据绑定 | `design.flow.bind` | `T27.03` |  |
| J18 | 4. 排序选择 | `design.content.sort` | `T27.04` |  |
| J18 | 5. 编辑校验 | `design.content.validate` | `T27.05` |  |
| J18 | 6. 空/错误 | `design.content.empty` | `T27.06` |  |
| J18 | 7. 生成 | `preview.matrix.generate` | `T27.08` |  |
| J19 | 1. 添加成员 | `project.workspace.add` | `T02.09` |  |
| J19 | 2. 同名文件导航 | `file.open.member` | `T02.09` |  |
| J19 | 3. 切活动包/Kit/feature | `project.config.kit` | `T02.10` |  |
| J19 | 4. 依赖变更审阅 | `project.config.dependencies` | `T02.11` |  |
| J19 | 5. 离线构建 | `build.reproduce.offline` | `T16.10` |  |
| J20 | 1. 前置任务 | `build.exec.task` | `T01.03` |  |
| J20 | 2. 后台就绪 | `panel.tasks` | `T01.03` |  |
| J20 | 3. 生成 | `preview.matrix.generate` | `T15.07` |  |
| J20 | 4. headless 检查/测试 | `build.reproduce.headless` | `T16.11` |  |
| J20 | 5. 导入报告 | `test.report.import` | `T21.07` |  |
| J20 | 6. 比对修订 | `vcs.change.review` | `T20.02` |  |
| J21 | 1. 选进程 |  | `T00.09` | 该步骤在本轮原型里没有可用入口分支，入口由归属任务一并确定 |
| J21 | 2. 授权采集 | `tools.workflow.profile` | `FUTURE` |  |
| J21 | 3. 复现 | `test.profile.reproduce` | `FUTURE` |  |
| J21 | 4. 取消/符号化 | `test.profile.cancel` | `FUTURE` |  |
| J21 | 5. 定位 | `test.profile.locate` | `FUTURE` |  |
| J21 | 6. 修复 | `code.edit.page` | `FUTURE` |  |
| J21 | 7. 对比 | `test.profile.compare` | `FUTURE` |  |
| J22 | 1. 选环境 | `project.remote.select` | `FUTURE` |  |
| J22 | 2. 信任/挂载预览 | `project.remote.trust` | `FUTURE` |  |
| J22 | 3. 连接 | `project.remote.connect` | `FUTURE` |  |
| J22 | 4. 编辑 | `code.edit.page` | `FUTURE` |  |
| J22 | 5. 构建测试 | `build.exec.build` | `FUTURE` |  |
| J22 | 6. 远程部署 | `project.remote.deploy` | `FUTURE` |  |
| J22 | 7. 断线 | `project.remote.disconnect` | `FUTURE` |  |
| J22 | 8. 重连 | `project.remote.reconnect` | `FUTURE` |  |
| J22 | 9. 停止 | `project.remote.stop` | `FUTURE` |  |
| J23 | 1. coder 工作区 | `tools.ai.collaboration.workspace` | `T23.09` |  |
| J23 | 2. 授权上下文 | `tools.ai.context` | `T23.03` |  |
| J23 | 3. share 候选/证据 | `tools.ai.collaboration.share` | `T23.09` |  |
| J23 | 4. reviewer | `tools.ai.collaboration.review` | `T23.10` |  |
| J23 | 5. 拒绝/修订 | `tools.ai.collaboration.revise` | `T23.10` |  |
| J23 | 6. 重审 | `tools.ai.collaboration.review` | `T23.10` |  |
| J23 | 7. 人工接受 | `vcs.change.review` | `T23.06` |  |
| J24 | 1. 提取键 | `project.config.language` | `T24.04` |  |
| J24 | 2. 翻译/长文本/RTL 预览 | `project.config.translate` | `T24.04` |  |
| J24 | 3. 空/错误数据 | `design.content.empty` | `T24.06` |  |
| J24 | 4. 校验 | `design.content.validate` | `T13.09` |  |
| J24 | 5. 提交取消 | `design.content.submit` | `T13.09` |  |
| J24 | 6. 双模式运行 | `run.app.start` | `T26.08` |  |
| J25 | 1. 导入快捷键/配置 profile | `tools.settings.import` | `T24.09` |  |
| J25 | 2. 审阅差异 | `tools.settings.profile` | `T24.09` |  |
| J25 | 3. 启动失败 | `run.app.start` | `T01.07` |  |
| J25 | 4. 禁扩展/二分 | `tools.extension.toggle` | `T22.05` |  |
| J25 | 5. 凭据撤销 | `tools.settings.credentials` | `T01.09` |  |
| J25 | 6. 恢复 | `file.open.recover` | `T03.06` |  |
| J26 | 1. 选包/运行配置 | `debug.native.launch` | `FUTURE` |  |
| J26 | 2. 双进程调试 | `debug.native.dual` | `FUTURE` |  |
| J26 | 3. 支持的内存/断点检查 | `debug.inspect.memory` | `FUTURE` |  |
| J26 | 4. 符号映射 | `debug.native.symbols` | `FUTURE` |  |
| J26 | 5. 停止 | `debug.session.stop` | `FUTURE` |  |

## 每条旅程的四个方面

路线图要求每条旅程都含**正常流程、取消/错误、恢复、键盘入口**。上表把它们各自**定位到具体步骤**：
单元格是「步骤序号 + 步骤文字」，即在那一刻要验证的东西。逐条的具体判据记在数据源的 `checks` 中
（每条旅程四条），本文件不重复展开以免两处漂移；要点如下：

- **正常流程**：旅程的主链路，必须在目标 profile 上真实跑通，不以单模块通过代替。
- **取消/错误**：取消、非法输入、权限拒绝或服务失效时的结果必须准确，不能表现为空成功。
- **恢复**：中断之后能继续，并且能证明"继续的是同一份内容"（修订、来源、最后有效模型）。
- **键盘入口**：该步骤有命令 id；快捷键层级与焦点策略由 T05.03/T05.05 实现。

## 未覆盖项与责任人

- **没有任何入口已实现。** 面板 id 与命令组来自资源清单（`resources/host/icons/manifest.json`），
  叶项名是本走查的原型决定，归属任务列给出实现者；`--status` 也不会报告这些入口可用。
- **命令组到模式的完整映射未定。** 本项只保证"旅程用到的命令叶项落在已声明分组之下"；模式工具栏
  具体放哪些命令由 T05.02 的菜单树与 T04.07 的上下文工具栏决定。
- **三条未来扩展旅程（J21/J22/J26）不进入正式版验收。** 它们的步骤仍有入口设计与归属（标为
  `FUTURE`），但没有任何"已实现"的含义。
- **键盘入口只到"有命令 id"这一层。** 实际按键、冲突检测与用户覆盖属于 T05.03。
- **人工/设备相关的证据不在本项。** 图像、DPI、可访问性与设备场景的验收记录属于 T00.07 与各旅程
  的收敛里程碑。
