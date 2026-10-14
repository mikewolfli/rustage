# rustage

**基于 [rust-widgets](https://github.com/mikewolfli/rust-widgets) 的 Rust
桌面可视化 UI 设计器与一体化开发工作台。**

[English](README.md) | [简体中文](README.zh-CN.md)

[![许可证：MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 项目状态

rustage 目前处于**设计与规划阶段**。仓库现已包含最小 Rust 可执行程序骨架，但
**尚无可运行的可视化设计器或已发布安装包**。该骨架固定了唯一的第三方依赖
[rust-widgets](https://github.com/mikewolfli/rust-widgets)（固定修订、`desktop` profile）
与固定的 Rust 工具链，并提供可运行探针核验依赖的布局、生成、事件与解绑契约；
它没有任何设计器窗口。

以下能力均为规划，不表示已经实现或通过平台验证。
完整路线图包含 29 个功能域、262 项实施任务、20 个里程碑、26 条用户旅程及对应验收标准。
其中正式版为 28 个功能域、251 项任务、23 条旅程；高级原生/复合调试、内置性能剖析、
远程/容器开发与远程部署单列未来扩展，不进入正式版依赖或发布门禁。

## 产品方向

- 可视化编辑页面、组件、布局、属性、CSS 样式和资源。
- 设计目标应用的菜单、共享动作、工具栏、弹窗和集合控件内容。
- 连接类型化数据、事件、状态、导航与 Rust 业务逻辑。
- 画布、对象树、源码视图和诊断共享同一工程上下文。
- 有效 UI 修改即时预览，不为每次界面修改重新编译 Rust。
- 确定性生成 Rust 源码，并保护显式声明的手写区域。
- 通过宿主服务集成编辑、语言服务、构建运行、调试、测试、版本控制和部署。
- AI 为可选、明确授权的变更提议工具；正常开发不依赖账号、网络或 AI。

## 桌面宿主与多目标产物

设计器本身仅在桌面运行，macOS、Windows、Linux 宿主需分别验证。
生成目标 profile 与操作系统、Rust target triple 是不同维度：

| 生成配置 | JSON/CSS 运行时产物（模式 1） | 静态 Rust 产物（模式 2） |
|---|---|---|
| `desktop` | 规划支持 | 规划支持 |好，二好了，我准备走了啊。6 点半了，干嘛多行的，的重新玩中英人民社区网英文。
| `tablet` | 规划支持 | 规划支持 |
| `mobile` | 规划支持 | 规划支持 |
| `mini` | 不适用 | 规划支持 |
| `embedded` | 不适用 | 规划支持 |

实际支持以库契约、工具链、SDK 和目标测试为准。桌面模拟不能代替真实设备认证。

### 分级热更新设计

1. **UI 与已有绑定**：校验后应用 JSON/CSS/模型变化。
2. **受管理 Rust 业务逻辑**：增量构建候选，通过版本化协议和显式状态迁移替换独立逻辑进程。
3. **不兼容变化**：明确要求重启，或经确认重置状态。

这不是任意 Rust 代码在进程内原地替换。开发期 IPC、文件监视等服务不得进入受限发布产物。

## 文档入口

| 文档 | 用途 |
|---|---|
| [文档索引](docs/README.md) | 阅读顺序与文档边界 |
| [设计白皮书](docs/plan/whitepaper.md) | 架构、产物模式与热更新方案 |
| [项目原则](docs/plan/principle.md) | 工程约束与评审准则 |
| [完整路线图](docs/plan/roadmap.md) | 产品模块、菜单、任务、里程碑与验收 |
| [支持矩阵](docs/plan/support-matrix.md) | 三宿主、五 profile 与逐格证据级别，及其数据文件 |
| [决策登记](docs/plan/decisions.md) | Q1–Q6 与工程格式、生成入库、升级、组件表达策略，以及仍未决定的取值 |
| [公共契约](docs/plan/contracts.md) | 唯一可写文档模型、变更集、权限、版本、诊断与源映射 |
| [旅程走查](docs/plan/journeys.md) | 七个模式原型，以及路线图每条旅程的逐步入口与归属任务 |
| [基准环境](docs/plan/benchmark.md) | 固定数据集、硬件基线、测量定义与性能场景门槛 |
| [需求追踪与门禁](docs/plan/traceability.md) | 需求到旅程、归属任务、门禁与测量场景的追踪 |
| [图标与资源规划](docs/plan/resources.md) | 资源放置、图标集成、主题与平台打包验收 |
| [能力规划依据](docs/plan/capability-reference.md) | 工作流需求、协议依据与未来扩展边界 |
| [GitHub 管理说明](docs/github.md) | 仍需仓库所有者配置的设置与 CI |

社区文档与 GitHub 模板以英文为主，设计文档继续使用中文。
项目状态或范围变化时，中英文 README 应同步更新。

## 仓库结构

```text
rustage/
├── .github/              # Issue 表单、PR 模板、归属与 CI
├── docs/
│   ├── plan/             # 原则、白皮书、路线图
│   ├── README.md         # 文档索引
│   └── github.md         # 仓库管理说明
├── src/                  # 可执行入口、CLI 边界与依赖契约探针
├── Cargo.toml            # Rust 包配置
├── Cargo.lock            # Rust 依赖锁定文件
├── rust-toolchain.toml   # 固定的 Rust 工具链
├── tools/                # 无第三方依赖的文档检查及测试
├── README.md             # 英文概览
├── README.zh-CN.md       # 中文概览
├── CONTRIBUTING.md       # 贡献流程
├── CODE_OF_CONDUCT.md    # 社区规范
├── SECURITY.md           # 漏洞报告政策
├── SUPPORT.md            # 问题与支持边界
├── CHANGELOG.md          # 经确认的仓库变更
└── LICENSE               # MIT 许可证
```

## 开始参与

```sh
git clone https://github.com/mikewolfli/rustage.git
cd rustage
cargo run
cargo run -- --help
cargo run -- --probe-apis target/probe   # 写出 target/probe/api-probes.json；有缺口时退出码非零
cargo run -- --probe-host target/host    # 写出 target/host/host-readiness.json；有检查未就绪时退出码 4
cargo test
node --test tools/check-docs.test.mjs
node tools/check-docs.mjs
```

Rust 骨架使用 2024 edition，第三方依赖是固定修订的 `rust-widgets`（冷构建时由 Cargo 联网拉取），
以及仅 unix 目标上的 `libc`（按需用于进程组信号，解析结果已在 `Cargo.lock` 中）；Rust 工具链由
`rust-toolchain.toml` 固定。文档检查需要
**Node.js 22 或更新版本**及 Git，无 npm 依赖。当前可执行程序报告项目状态、支持帮助与版本
选项、渲染图标条带、运行依赖契约探针、检查宿主自身的平台服务，并启动有界的基线桌面会话。
尚未实现设计器本身：文档模型、画布、属性面板、预览与生成器都还不存在。

## 贡献与支持

提交改动前请阅读[贡献指南](CONTRIBUTING.md)。
缺陷、功能提议、设计决策和目标兼容性证据使用对应 Issue 表单；
一般问题的提交方式见[支持说明](SUPPORT.md)。

社区参与遵循[行为准则](CODE_OF_CONDUCT.md)。
不要在公开 Issue 中披露漏洞或机密，请按[安全政策](SECURITY.md)报告。

## 许可证

本项目采用 [MIT License](LICENSE)。
第三方依赖、资源、字体和导入模板保留各自许可证，发行前须单独审查。
