# 需求追踪、发布范围与最小自动门禁

> 数据源：[`traceability.json`](traceability.json)。本文件的表格由
> [`tools/traceability/check-traceability.mjs`](../../tools/traceability/check-traceability.mjs)
> 逐行对照该文件校验；同一脚本核对每条需求点名的旅程与任务在[路线图](roadmap.md)中真实存在、
> 每个门禁要么在 `.github/workflows/docs.yml` 里真的被执行、要么写明理由与归属，
> 并核对命令命名空间与资源清单一致。
> 依据：[项目原则](principle.md) #22/#25/#38、[路线图](roadmap.md) §1.2/§1.4/§35、[白皮书](whitepaper.md) §2、
> [旅程走查](journeys.md)、[基准环境](benchmark.md)。
> 状态：**16 条需求各自都有归属任务、门禁与测量场景；21 个门禁中 16 个已在 CI 中执行、5 个为人工门禁
> 并写明理由；基线是 T00.01–T00.09。没有任何需求处于"无人负责"状态，也没有声明了却不运行的门禁。**

## 这份文件回答什么

T00.08 的最低验收是「无未归属需求，无依赖环，基线可复核」。三者在这里分别落成三件可检查的事：

1. **无未归属需求**：每条需求都带出处（白皮书 §2 或路线图 §1.2）、至少一条旅程、至少一个归属
   任务、至少一个门禁。缺任何一项，检查失败。
2. **无依赖环**：任务依赖由 `tools/check-docs.mjs` 在路线图上检查（缺依赖与环都会失败）；本文件
   只引用真实存在的任务 id，不新增第二份依赖图。
3. **基线可复核**：MS00 的基线是 T00.01–T00.09；每项任务的可复核证据在
   [`log-20261013-4.md`](../log/log-20261013-4.md) 里，包含命令、输出、修订与未覆盖项。

## 门禁怎么读

**方式**列只有两个取值：`ci` 表示该命令必须出现在 `.github/workflows/docs.yml` 里（检查脚本逐字
比对，声明了却不运行会失败）；`manual` 表示需要人手或设备，必须写明理由与归属。Rust 侧门禁目前
全部是 `manual`：CI 里还没有 Rust 工具链与 git 依赖拉取，这一点在理由列里写明，而不是假装它们
在自动运行。

## 需求 → 旅程 → 任务 → 门禁 → 场景

| 需求 | 陈述 | 出处 | 旅程 | 归属任务 | 门禁 | 测量场景 |
| --- | --- | --- | --- | --- | --- | --- |
| G1 | 所见即所得：改一处立即看到，不重编译 | whitepaper §2 G1 | J01, J06, J07 | `T14.01`, `T14.02`, `T18.05` | rust-test, benchmark | design-update-100, hot-swap-idle |
| G2 | 产物是干净的本项目代码，可独立构建 | whitepaper §2 G2 | J08, J16 | `T15.02`, `T15.07`, `T16.08` | rust-test, benchmark | editor-2m |
| G3 | AI 友好：反馈回路短，结构可生成与修改 | whitepaper §2 G3 | J12, J23 | `T23.03`, `T23.04`, `T23.05` | rust-test, journeys | ai-session-budget |
| G4 | 覆盖全部 profile：desktop/tablet/mobile/mini/embedded | whitepaper §2 G4 | J08 | `T00.03`, `T15.02`, `T16.08` | support-matrix, rust-test | editor-2m |
| G5 | 不说谎：不可用能力在画布上标示，不在生成时才失败 | whitepaper §2 G5 | J04, J15, J24 | `T09.01`, `T21.02`, `T22.02` | contracts, journeys | design-update-100 |
| B1 | 宿主仅桌面，分别支持并实测 macOS/Windows/Linux | roadmap §1.2 宿主 | J16 | `T00.03`, `T01.08` | support-matrix | design-update-100 |
| B2 | profile 与 OS/target triple 分开建模 | roadmap §1.2 生成配置 | J08, J19 | `T02.10`, `T15.01` | support-matrix, check-docs | editor-2m |
| B3 | 模式 1/模式 2 并存；mini/embedded 只模式 2 | roadmap §1.2 产物 | J08, J24 | `T15.02`, `T16.08` | support-matrix, decisions | design-update-100 |
| B4 | 有效模型是唯一事实来源：JSON 结构、CSS 外观、稳定 NodeId、版本化变更集 | roadmap §1.2 模型 | J02, J06, J14 | `T00.05`, `T02.01`, `T03.01` | contracts, rust-test | churn-100 |
| B5 | 布局复用库语义：拖拽结果必须可表达为真实布局约束 | roadmap §1.2 布局 | J04 | `T00.01`, `T10.01`, `T10.02` | rust-test, benchmark | design-update-100 |
| B6 | 能力查询带版本的库契约；不维护猜测的控件支持表，不静默降级 | roadmap §1.2 能力 | J08, J15 | `T00.01`, `T09.01`, `T12.04` | rust-test, contracts | design-update-100 |
| B7 | 生成区受保护，手写 Rust 可编辑；不承诺把任意 Rust 反推成 UI | roadmap §1.2 源码 | J06 | `T00.04`, `T06.06`, `T15.01` | decisions, contracts | design-update-100 |
| B8 | 热更新分级：UI 即时更新，受管理 Rust 增量替换，不兼容则明确重启 | roadmap §1.2 热更新 | J07, J09 | `T14.02`, `T18.05`, `T18.06` | rust-test, benchmark | hot-swap-idle |
| B9 | 文件/语言服务/构建/调试/版本控制/网络属于宿主，widget 不承载系统服务 | roadmap §1.2 服务 | J03, J10, J20 | `T01.07`, `T07.01`, `T16.02` | rust-test | ipc-roundtrip |
| B10 | 工程代码、构建脚本、终端、设备部署与外部服务具有真实副作用，工作进程不是沙箱 | roadmap §1.2 权限 | J12, J20, J23 | `T00.05`, `T01.05`, `T01.09` | contracts, rust-test | ipc-roundtrip |
| B11 | 无 AI、无账号、无网络仍能完成本地工程闭环 | roadmap §1.2 人工流程 | J01, J03, J16 | `T16.10`, `T22.05`, `T23.01` | journeys, check-docs | test-loop |

## 门禁清单

| 门禁 | 命令 | 覆盖 | 方式 | 理由 / 归属 |
| --- | --- | --- | --- | --- |
| rust-test | `cargo test` | Rust 单元测试、契约不变量与库层探针的断言 | manual | 需要 Rust 工具链与 git 依赖拉取，尚未在 CI 实测（T00.08 / CI 维护者） |
| rust-fmt | `cargo fmt --check` | 格式 | manual | 同上（T00.08 / CI 维护者） |
| rust-clippy | `cargo clippy --all-targets` | 静态检查；不得新增警告 | manual | 同上（T00.08 / CI 维护者） |
| probe-apis | `cargo run -- --probe-apis target/probe` | 固定修订上的库契约探针；有缺口时退出码为 4（`ExitCode::Reported`，与 `--probe-host` 同一个码） | manual | 缺口存在时该命令刻意失败，需人工判读（T00.08 / CI 维护者） |
| check-docs-test | `node --test tools/check-docs.test.mjs` | 文档检查器自身的负例 | ci | — |
| check-docs | `node tools/check-docs.mjs` | Markdown 链接/空白/必需文件/路线图依赖与计数 | ci | — |
| resources-test | `node --test tools/resources/check-resources.test.mjs` | 资源检查器负例 | ci | — |
| resources | `node tools/resources/check-resources.mjs` | 图标清单、主题角色、许可证与打包定义 | ci | — |
| support-matrix-test | `node --test tools/support-matrix/check-support-matrix.test.mjs` | 矩阵检查器负例 | ci | — |
| support-matrix | `node tools/support-matrix/check-support-matrix.mjs` | 三宿主×五 profile 的证据级别与文档一致 | ci | — |
| decisions-test | `node --test tools/decisions/check-decisions.test.mjs` | 决策登记检查器负例 | ci | — |
| decisions | `node tools/decisions/check-decisions.mjs` | 决策条目完整性、悬置问题归宿、白皮书 §17.7 一致 | ci | — |
| contracts-test | `node --test tools/contracts/check-contracts.test.mjs` | 契约检查器负例 | ci | — |
| contracts | `node tools/contracts/check-contracts.mjs` | 写入入口枚举、公开字段、不变量用例存在性、白皮书 §12.5 一致 | ci | — |
| journeys-test | `node --test tools/journeys/check-journeys.test.mjs` | 旅程检查器负例 | ci | — |
| journeys | `node tools/journeys/check-journeys.mjs` | §34 旅程逐步入口与归属、模式面板可发现性 | ci | — |
| fixtures-test | `node --test tools/fixtures/check-fixtures.test.mjs` | 数据集定义与生成器用例 | ci | — |
| fixtures-verify | `node tools/fixtures/generate-fixtures.mjs --verify` | fixture 生成可复现（流式 SHA-256 两次一致） | ci | — |
| benchmark-test | `node --test tools/benchmark/check-benchmark.test.mjs` | 基准检查器负例 | ci | — |
| benchmark | `node tools/benchmark/check-benchmark.mjs` | 场景门槛与路线图 §35.2 一致、数据集双向对账 | ci | — |
| device-acceptance | `（人工）目标设备与 SDK 上的构建与运行` | 平台认证级别：能力查询→生成→target check→target build→运行→真机 | manual | 缺设备/SDK/辅助环境；按支持矩阵逐格标 blocked 或 unverified（T00.08 / CI 维护者） |

## 发布范围与基线

| 范围项 | 取值 |
| --- | --- |
| 基线任务 | `T00.01`, `T00.02`, `T00.03`, `T00.04`, `T00.05`, `T00.06`, `T00.07`, `T00.08`, `T00.09` |
| 正式版范围 | 28 个功能域、251 项任务、23 条旅程 |
| 说明 | MS00 的基线是 T00.01–T00.09 九项前置任务；正式版范围以路线图 §1.4 为准，未来扩展（F28 与 J21/J22/J26）不进入完成率。 |

## 命令命名空间

| 命令命名空间 | 来源 |
| --- | --- |
| build | resources/host/icons/manifest.json |
| code | resources/host/icons/manifest.json |
| debug | resources/host/icons/manifest.json |
| design | resources/host/icons/manifest.json |
| edit | resources/host/icons/manifest.json |
| file | resources/host/icons/manifest.json |
| help | resources/host/icons/manifest.json |
| nav | resources/host/icons/manifest.json |
| preview | resources/host/icons/manifest.json |
| project | resources/host/icons/manifest.json |
| run | resources/host/icons/manifest.json |
| test | resources/host/icons/manifest.json |
| tools | resources/host/icons/manifest.json |
| vcs | resources/host/icons/manifest.json |
| view | resources/host/icons/manifest.json |
| window | resources/host/icons/manifest.json |

命令 ID 采用 `域.动作`；这里的 16 个命名空间来自资源清单的资源分组。命令组到路线图 §3 菜单树、以及菜单叶项到命令测试的映射由 T05.02 落实，本项只保证命名空间不越界。
## 未覆盖项与责任人

- **需求列表是节选，不是白皮书全量。** 16 条覆盖 §2 的设计目标 G1–G5 与路线图 §1.2 的 11 条架构
  边界；principle.md 的 68 条原则是**评审准则**，不作为可归属的"需求"逐条登记，避免把准则拆成
  262 个任务之后互相对不上。
- **映射的正确性是评审问题。** 检查能拒绝"任务不存在""门禁不存在"，不能判断某个任务是否真的交付
  了那条需求；归属任务列就是复核入口。
- **Rust 门禁尚未自动化。** `cargo test`/`fmt`/`clippy`/`--probe-apis` 仍是人工门禁；把它们搬进 CI
  需要先在 CI 实测 GUI profile 构建与 git 依赖拉取（记在 §下一步）。
- **命令叶项到命令测试的映射未定。** 本项只保证命名空间不越界；菜单叶项与命令 id、以及"每个叶项
  有命令测试"由 T05.02/T05.08 落实。
- **设备与平台认证仍是人工门禁。** 支持矩阵 15 格目前全部为 `build-verified`（三宿主 × 五
  profile），没有一格 `certified`；以 `support-matrix.md` 为准。缺设备时按矩阵标 `blocked`/
  `unverified`，不用人工门禁掩盖。
