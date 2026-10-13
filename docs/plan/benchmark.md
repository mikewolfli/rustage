# 基准环境与测量定义：数据集、硬件与门槛

> 数据源：[`benchmark.json`](benchmark.json)。本文件的表格由
> [`tools/benchmark/check-benchmark.mjs`](../../tools/benchmark/check-benchmark.mjs) 逐行对照该文件
> 校验；同一脚本解析路线图 §35.2 的场景表并逐行比对**场景文字、门槛文字与验收阶段**（改一边而漏
> 另一边会失败），核对场景引用的数据集在 [`fixtures/spec.json`](../../fixtures/spec.json) 中真实
> 存在，并与该文件中的反向引用（fixture 的 `measurement` 列出的场景 id）互相对账。
> 依据：[项目原则](principle.md) #22/#24/#25、[路线图](roadmap.md) §35.2、[支持矩阵](support-matrix.md)、
> [白皮书](whitepaper.md) §16.5。
> 状态：**数据集规模、硬件基线、测量定义与 18 个场景的门槛已固定；没有任何一项已测量**——
> 设计器尚未实现，本节交付的是可复核的测量契约，不是性能结论。

## 这份文件回答什么

路线图 §35.2 的门槛后面写着"待 MS00 校准"：在那之前，那些数字只是建议。T00.07 的验收就是
"固定数据规模、硬件和测量定义"，也就是把下面四件事钉死，使后续任何一次测量都能被复核：

1. **数据规模**：每个场景用哪个 fixture、fixture 里有多少行/多少节点，写在 `fixtures/spec.json`。
   规模太大不能入库，因此 fixture 由生成器确定性产出，检查脚本用流式 SHA-256 证明两次生成一致。
2. **硬件与环境**：本机型号、OS、CPU、核心数、内存、工具链与 Node 版本，连同采集命令。
3. **测量定义**：warmup、重复次数、统计量、起止点、排除项与噪声控制；只报最佳一次视为无效。
4. **门槛出处**：每个场景的门槛文字与路线图逐字一致，**不允许为通过验收悄悄放宽**（路线图原文）。

## 数据集

生成命令（产物不入库，见 `fixtures/spec.json` 的 `note`）：

```sh
node tools/fixtures/generate-fixtures.mjs --output target/fixtures   # 按 spec 写出全部 fixture
node tools/fixtures/generate-fixtures.mjs --verify                   # 只算哈希，证明可复现
```

五个 fixture：`normal`（日常闭环）、`large-file`（1 万/5 万/50 万/200 万行与超长行、Unicode）、
`multi-page`（12 页 480 节点）、`restricted-target`（仅模式 2 的目标）、`failing`（解析失败、缺失
资源、编译错误）。

## 硬件与环境基线

| 项 | 取值 |
| --- | --- |
| host | Mac16,12 |
| os | macOS 15.7.3 (24G419) |
| arch | aarch64 |
| cpu | Apple M4 |
| cores | 10 |
| memoryBytes | 17179869184 |
| rustc | 1.98.1 (48a229cea 2026-09-01) |
| node | v24.11.1 |

采集命令：

- `sw_vers`
- `sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize hw.model`
- `rustc --version`
- `node --version`
- `uname -m`

这只是设计器宿主基线。输出目标宿主需要各自设备与 SDK，状态见 docs/plan/support-matrix.md；不得用宿主数字代替目标数字。

## 测量定义

| 项 | 定义 |
| --- | --- |
| state | 冷启动与热态分别记录，不混合 |
| warmup | 3 |
| repetitions | 30 |
| statistic | 同一批次内取 p50 与 p95，报告最小值无意义 |
| startPoint | 用户操作或命令派发的入口（含校验，不含等待构建/网络/用户输入） |
| endPoint | 可见结果出现：绘制帧提交或问题列表/状态栏更新 |
| excluded | 编译时间, 网络等待, 用户在对话框里的思考时间, 测试夹具生成时间 |
| noise | 同一批次内不得混入后台任务；记录进程 CPU 占用作为旁证 |
| reporting | 每次测量记录：fixture id、修订（git rev）、工具链、warmup/重复次数、原始分布与统计量；只报最佳一次视为无效 |

## 状态词汇

| 状态 | 含义 |
| --- | --- |
| measured | 有原始数据、修订与日期，可复核 |
| unmeasured | 本仓库还没有测量记录 |
| blocked | 缺设备/SDK/辅助环境，无法测量 |
| future | 属于不进入正式版的未来扩展 |

## 场景清单

| 编号 | 场景 | 数据集 | 指标 | 统计 | 门槛 | 验收阶段 | 状态 | 归属 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| design-update-100 | 100 节点单属性/CSS 更新 | `normal` | 提交到可见帧 | p95 ≤ 100 ms | 提交到可见帧 p95 ≤ 100 ms；不启动 cargo | MS06 | unmeasured | `T14.02` |
| tree-1000 | 1000 节点树与画布 | `normal`, `multi-page` | 选择/展开可见反馈 | p95 ≤ 100 ms | 选择/展开可见反馈 p95 ≤ 100 ms；不全量重建无关实例 | MS05/MS18 | unmeasured | `T09.03` |
| editor-10k | 1 万行编辑/滚动 | `large-file` | 可见反馈 / 绘制 CPU | p95 ≤ 50 ms / ≤ 16.7 ms | 可见反馈 p95 ≤ 50 ms；绘制 CPU p95 ≤ 16.7 ms | MS04 | unmeasured | `T06.08` |
| editor-2m | 百万行/超长行打开查看 | `large-file` | 打开可取消 / UI 心跳 | p95 ≤ 100 ms | 打开可取消；UI 心跳 p95 ≤ 100 ms；分级降级可见 | MS04/MS18 | unmeasured | `T06.07` |
| quick-open | 命令面板/快速打开 | `multi-page` | 本地已索引结果反馈 | p95 ≤ 100 ms | 本地已索引结果反馈 p95 ≤ 100 ms；索引未就绪有状态 | MS03/MS04 | unmeasured | `T05.04` |
| search-project | 工程搜索/索引 | `large-file` | 取消反馈 / 实际退出 | p95 ≤ 200 ms（退出单独测量） | 异步/有界；取消反馈 p95 ≤ 200 ms，实际退出单独测量 | MS04 | unmeasured | `T08.02` |
| ipc-roundtrip | 本地业务 IPC | `normal` | 排除业务耗时后的往返 | p95 ≤ 20 ms | 排除业务耗时后往返 p95 ≤ 20 ms，载荷大小固定 | MS09 | unmeasured | `T17.03` |
| hot-swap-idle | 无长任务、1 MiB 状态切换 | `normal` | 暂停 | p95 ≤ 300 ms | 暂停 p95 ≤ 300 ms；队列满/超时可见 | MS09 | unmeasured | `T18.05` |
| hot-swap-rust | Rust 源码到新行为 | `normal` | 冷/热 build、启动、迁移、切换 | p50/p95，不承诺总耗时 | 分开记录冷/热 build、启动、迁移、切换 p50/p95，不承诺毫秒级总耗时 | MS09 | unmeasured | `T18.05` |
| churn-100 | 100 轮绑定/预览/逻辑替换 | `normal` | 订阅/句柄/线程/进程残留与稳态内存趋势 | 无持续增长 | 订阅/句柄/线程/进程不残留，稳态内存无持续增长趋势 | MS06/MS09 | unmeasured | `T14.08` |
| terminal-output | 终端/日志高速输出 | `normal` | 有界缓存/背压与键入延迟 | 不阻塞键入、不隐藏丢弃 | 有界缓存/背压/可暂停查看，不阻塞键入，不隐藏丢弃 | MS07 | unmeasured | `T18.06` |
| debug-large-values | 调试大变量/慢适配器 | `normal` | 惰性读取/可取消与 UI 心跳 | 达基线、不无界展开 | 惰性读取、可取消、UI 心跳达基线，不无界展开 | MS11 | unmeasured | `T19.05` |
| cold-start | 冷启动与首次工程打开 | `normal`, `multi-page` | 宿主/索引/服务就绪分别记录 | MS00 定预算后实测 | MS00 定预算后实测，分别记录宿主/索引/服务就绪，不编造秒数 | MS18 | unmeasured | `T01.01` |
| workspace-switch | 多根/feature/Kit 切换 | `multi-page` | 根/包数量、分析重建与诊断延迟 | 旧结果不盖新配置 | 记录根/包数量、分析重建与诊断延迟；旧结果不盖新配置 | MS04/MS18 | unmeasured | `T02.10` |
| test-loop | 连续测试 | `normal` | 结果体积与定向重跑成本 | 不拖慢前台 | 测结果体积/定向重跑成本，不以自动后台任务拖慢前台 | MS14/MS18 | unmeasured | `T21.05` |
| profile-overhead | 内置性能剖析（未来扩展） | `normal` | 采集开销/符号化/报告规模 | FUTURE | 测采集开销/符号化/报告规模，不用缺失数据伪造结论 | FUTURE，不阻塞正式版 | future | `T00.09` |
| remote-reconnect | 远程断线/重连（未来） | `normal` | 固定延迟/丢包/文件规模 | FUTURE | 固定延迟/丢包/文件规模；保存不假成功，取消与实际远端退出分别测量 | FUTURE，不阻塞正式版 | future | `T00.09` |
| ai-session-budget | 双 AI 会话与共享材料 | `normal` | 上下文/输出/工具/预算上限 | MS16/MS18 | 上下文/输出/工具/预算有上限；取消后不提交，旧候选/评审永不覆盖当前 | MS16/MS18 | unmeasured | `T23.09` |
## 未覆盖项与责任人

- **没有任何测量。** 18 个场景全为 `unmeasured` 或 `future`；`measured` 状态必须附原始数据、修订
  与日期，检查脚本会拒绝缺少这些字段的测量声明。
- **受限目标与设备场景缺环境。** 需要目标 SDK、真机或辅助技术环境的场景只能标 `blocked`；支持
  矩阵里 15 格除 macOS 宿主构建外全部未验证，这一点不会被这里的数字掩盖。
- **硬件基线只是设计器宿主。** 目标侧性能必须用目标设备测量；本文件不把宿主数字当作目标数字
  （原则 #24）。
- **fixture 的"期望"尚未自动化。** `expect` 描述的是每个 fixture 应当呈现什么（解析通过/失败、
  cargo check 通过/失败），但真正跑 `cargo check` 需要工具链与网络，属于各收敛里程碑；这里不
  声称已验证。
- **门槛校准尚未完成。** 路线图 §35.2 的门槛是"建议门槛"；校准需要真实测量，而测量需要实现。
  本文件的作用是让校准结果有地方可比对，而不是假装已经校准。
