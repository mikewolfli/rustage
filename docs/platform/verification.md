# 需要真机与真设备验证的清单

> 目的：把散在各处"必须有真机/真设备才能取得证据"的项集中到一处，逐项写清**缺什么**、**在哪台机器上
> 跑什么**、**结果写回哪里**，并标出**已经有真机证据的项**。
> 依据：[项目原则](../plan/principle.md) #22（失败必须显式可诊断）、#24（用目标构建验证兼容性）、
> #25（每项结论附可复核证据）、#45（平台探测必须验证真实语义）、#48（使用跨平台路径 API）。
> 数据来源（本文件不复制它们的结论，只做索引与状态汇总）：
> [支持矩阵](../plan/support-matrix.md)、[探针记录](../plan/host-probe.md)、
> [基准环境](../plan/benchmark.md)、[资源与鸿蒙打包问题](../issues/issue-20261013.md)、
> [第七次开发记录](../log/log-20261015-1.md)（MS00/MS01 验收复核）、
> [第八次开发记录](../log/log-20261015-2.md)（Windows 交叉构建与 Wine 实测）、
> [第九次开发记录](../log/log-20261015-3.md)（依赖 re-pin 与两处探针更新）。

## 0. 状态词汇与三条口径

| 状态 | 含义 |
|---|---|
| ✅ **已验证** | 在**真机/真设备/真实桌面会话**上取得了证据，并写明证据位置；不是"看起来能跑" |
| ⏳ **待真机** | 缺机器、缺显示会话、缺人（屏幕前确认）；补法与命令写在同一行 |
| ⛔ **阻塞** | 还缺 SDK、签名身份或设备，且**有明确的解锁条件**（不是"以后再说"） |

三条口径（都来自本项目已经付出的代价）：

1. **模拟不能顶替真机。** Wine、状态后端（`PreviewOrStub`）、模拟器各能证明一部分，但不能证明
   "在那个 OS 上如何运行"——Wine 的剪贴板接的是 X11，状态后端连窗口都不合成。见
   [支持矩阵规则 6](../plan/support-matrix.md) 与 [第八次开发记录 §1](../log/log-20261015-2.md)。
2. **构建与运行是两件事。** 构建立即能证明（交叉编译即可，见 A18）；运行只能在该 OS 上证明。
   `build-verified` 与 `certified` 因此是两个不同的级别。
3. **一个人的一次确认也算证据，但必须写下来。** "窗口出现在屏幕上、点了它真的有反应"这类结论需要
   人在屏幕前，属于 A2；把它写成"已验证"之前必须先记下时间、机器与看到的输出。

## 已经被真机验证的项（一眼看）

| 编号 | 项 | 机器 / 环境 | 证据 |
|---|---|---|---|
| A1 | macOS 基线会话真实启动 | macOS 15.7.3 / Apple M4，后端 `cocoa`、模式 `NativeInteractive` | [log-20261013-4 §16](../log/log-20261013-4.md) |
| A3 | Linux 的 15 项探针 | 本机 linux x86_64，后端 `linux-state-backend`（状态后端） | [host-probe.md](../plan/host-probe.md) linux 行 |
| A4 | Linux **原生** GTK 会话 | 本机 linux x86_64 + X11 + GTK3，后端 `gtk` | [log-20261015-1 §7](../log/log-20261015-1.md) |
| A18 | Windows 交叉构建（`x86_64-pc-windows-gnu`） | 本机 linux，mingw-w64 GCC 13 | [log-20261015-2 §2](../log/log-20261015-2.md) |
| A20 | 宿主构建（macOS、Linux） | 两台机器各自的 `cargo build` | [log-20261013-4](../log/log-20261013-4.md)、[log-20261015-1 §11](../log/log-20261015-1.md) |
| A23 | macOS x86_64 交叉构建与 **Rosetta 运行** | 本机 macOS（Apple Silicon），`rustup target add x86_64-apple-darwin` | [log-20261016-4 §28](../log/log-20261016-4.md)：`cargo build --target x86_64-apple-darwin` 通过，`target/x86_64-apple-darwin/debug/rustage --version` 返回 `rustage 0.1.0` |
| A24 | 固定依赖 `rust_widgets` 的 `mini`/`embedded` profile 构建 | 本机 macOS，`--no-default-features --features <profile> --lib` | [log-20261016-4 §28](../log/log-20261016-4.md)：两个 profile 均 `cargo build` 通过，rlib 84,282,248 / 69,528,992 字节；生成物未链接 |
| A25 | Linux GTK 控件事件、键盘与真实 IME | 本机 linux x86_64 + X11 + GTK3 + fcitx5（激活 `pinyin`） | [log-20261019-1 §11](../log/log-20261019-1.md)：声明事件/绑定/多控件路由经真实控件进入 `--logic-worker`；`--keys` 得到 `abc`；`--ime` 连续 8 次得到 preedit 与 `commit: 你好`；`你好世界` 全字形截图 |

**只有这 7 项**。其余每一项都需要一台机器、一块设备、一次人工确认，或一个 SDK；逐项见下。

## 1. 三桌面宿主（T01.01 启动、T01.02 平台服务、T01.08 探针）

| # | 项 | 需要什么 | 怎么补 | 状态 | 证据 / 缺口 |
|---|---|---|---|---|---|
| A1 | macOS 基线会话真实启动 | 一台 macOS + 图形会话 | `cargo run -- --launch --exit-after-ms 600` | ✅ 已验证 | [log-20261013-4 §16](../log/log-20261013-4.md)：后端 `cocoa`、模式 `NativeInteractive`、窗口 id 与几何为运行时返回值 |
| A2 | 窗口真的**出现在屏幕上并接受输入**（三桌面各一次） | 人在屏幕前 | `cargo run -- --launch`，用鼠标点窗口、关窗口，记录 `run()` 返回与日志 | ⏳ 待真机 | 本机 Linux 与 macOS 都只到"窗口被创建、循环进入"；合成与输入无人确认（[log-20261015-1 §12.1](../log/log-20261015-1.md)）。2026-10-19 已有 XTest 合成点击/拖拽、`--smoke-worker` 跨进程状态与 §11 的多控件事件路由证据（[log-20261019-1 §5.7、§11](../log/log-20261019-1.md)），键盘/IME 见 A16；但合成输入仍不等于人工确认 |
| A3 | Linux 默认构建的 15 项探针 | 一台 Linux + 任意会话 | `cargo run -- --probe-host target/host` | ✅ 已验证 | [host-probe.md](../plan/host-probe.md) linux 六组 `probed`；`12 ready / 1 gap / 2 not-verified` |
| A4 | Linux **原生** GTK 会话 | Linux + X11（本机有） | `cargo run --features linux-native -- --probe-host target/host` | ✅ 已验证 | [log-20261015-1 §7](../log/log-20261015-1.md)：后端 `gtk`、模式 `NativeInteractive`、窗口创建成功；re-pin 后重跑为 `13 ready / 0 gap / 2 not-verified`（[log-20261015-3](../log/log-20261015-3.md)） |
| A5 | Linux **Wayland** 原生会话 | Wayland 会话（本机 `WAYLAND_DISPLAY` 为空） | 在 Wayland 桌面里用上游 `wayland-native` feature 重跑探针 | ⏳ 待真机 | 上游用 `wayland-native` 选择该后端；本机没有 Wayland 合成器 |
| A6 | macOS 的 15 项探针 | 一台 macOS | `cargo run -- --probe-host target/host` | ⏳ 待真机 | [host-probe.md](../plan/host-probe.md) macOS 列全 `blocked` |
| A7 | Windows 的 15 项探针 | 一台 Windows | `cargo run -- --probe-host target/host` | ⏳ 待真机 | 同上；Wine 下的结果不计（[规则 6](../plan/support-matrix.md)） |
| A8 | Windows 上 4 项检查的受控程序 | **先改代码**，再要机器 | 把 `process-trees`/`project-trust`/`notifications`/`project-scope` 的受控程序从 `sh` 换成平台可用者（候选：`rustage` 自身） | ⏳ 待真机（前置是代码改造） | 否则这 4 格在真机上也会报**假缺口**（[log-20261015-2 §5](../log/log-20261015-2.md)） |
| A9 | macOS 原生剪贴板（`NSPasteboard`） | 一台 macOS | 探针的 `clipboard` 检查 | ⏳ 待真机 | Linux/X11 已 ✅（A4）；macOS 未测 |
| A10 | Windows 原生剪贴板 | 一台 Windows | 同上 | ⏳ 待真机 | Wine 下的 `clipboard -> ready` 走的是 X11 桥，不算（[log-20261015-2 §3](../log/log-20261015-2.md)） |
| A11 | 真实的 URL 交接（打开浏览器/编辑器） | 人 + 任意桌面 | 探针的 `url-handover` 检查刻意不做；需要一次人工操作并记录 | ⏳ 待真机 | 三平台都是 `not-verified`：探针不得打开用户的浏览器（[host-probe.md](../plan/host-probe.md)） |
| A12 | Windows 的 `cmd /C start` 交接 | 一台 Windows | `open_url` 的交接检查（宿主自己的用例已有） | ⏳ 待真机 | 目前只有 Wine 证据；映射本身由用例覆盖（[log-20261014-1 §6](../log/log-20261014-1.md)） |
| A13 | 非 unix 的进程树语义 | 一台 Windows | `process-trees` 检查；记录是 `DirectOnly` 还是 Job Object | ⏳ 待真机 | `Termination::DirectOnly`、`ProcessPresence` 的非 unix 分支只有代码与文档（[log-20261015-1 §12.2](../log/log-20261015-1.md)） |
| A14 | 系统凭据存储 | macOS 钥匙串 / Windows 凭据管理器 | `default_store()` 与 `network-services` 检查 | ⏳ 待真机 | 本机（与多数 Linux 会话）没有可用 keyring，走的是环境变量存储（[log-20261014-1 §27.6](../log/log-20261014-1.md)） |
| A15 | 真实 DPI 缩放与多屏 | 可切换缩放的桌面、多显示器 | 改系统缩放后跑 `--probe-host`，比较 `dpi_scale_factor` 与几何 | ⏳ 待真机 | `GDK_SCALE`/`GDK_DPI_SCALE` 有代码路径，无真实切换证据（[log-20261015-1 §12.1](../log/log-20261015-1.md)） |
| A16 | 编辑器绘制、IME、鼠标/触摸路径 | 真实显示会话 + 输入法 | 打开编辑器窗口，用鼠标选词、用 IME 输入 | ⏳ 部分 | T00.02 的未覆盖项。`--smoke` 的字段路径已验证：键盘 `abc`、真实 fcitx preedit/commit `你好`、`你好世界` 全字形截图（[log-20261019-1 §11](../log/log-20261019-1.md)）；依赖宿主 `grab_focus` workaround 与已激活的 `pinyin`。编辑器控件的绘制/选词/触摸仍未在真实编辑器窗口中验证（[log-20261013-4 §17](../log/log-20261013-4.md)） |
| A17 | 可访问性桥（AT-SPI/Narrator/VoiceOver） | 各平台辅助技术 | 用真实读屏软件读一遍工具栏与面板 | ⏳ 待真机 | 本仓库没有用真实辅助技术验证过；`accessibility` 目前只是库的能力位 |
| A18 | **Windows 交叉构建** | mingw-w64（本机已装） | `cargo build --target x86_64-pc-windows-gnu` | ✅ 已验证 | [log-20261015-2 §2](../log/log-20261015-2.md)：1m01s 通过，产出 `rustage.exe`；同目标 `clippy --all-targets` 亦通过 |
| A19 | Windows `x86_64-pc-windows-msvc` 构建 | MSVC CRT/SDK 或 `cargo-xwin` | `cargo build --target x86_64-pc-windows-msvc` | ⏳ 待真机 | 只有 `-gnu` 的构建证据（[log-20261015-2 §8](../log/log-20261015-2.md)） |
| A20 | macOS 与 Linux 宿主构建 | — | `cargo build` | ✅ 已验证 | macOS：[log-20261013-4](../log/log-20261013-4.md)；Linux：本机每轮 `cargo build`（[log-20261015-1 §11](../log/log-20261015-1.md)） |
| A21 | macOS 应用包结构、签名与公证 | Apple 开发者身份 + 公证流程 | F25（发行、更新、兼容、维护与稳定发布）里的打包与签名任务 | ⛔ 阻塞 | 需要证书与账号（[issue-20261013 §6](../issues/issue-20261013.md)） |
| A22 | 三平台干净用户环境的安装与启动 | 三台（或三个）干净环境 | 安装包 → 启动 → 检查系统图标/快捷方式/启动器 | ⏳ 待真机 | [issue-20261013 §6](../issues/issue-20261013.md) 的四平台验收表 |

## 2. 输出 profile 与设备（T00.03 的 15 格 + F18 多目标与部署）

[支持矩阵](../plan/support-matrix.md) 的 15 格中，2026-10-20 之后全部为 `build-verified`
（三宿主 × 五 profile）。逐格补法是"在某一宿主上为某一 profile 建立构建或运行证据"；构建通过
不等于运行，因此没有任何 profile 格写成 `certified`：

| # | 格 | 需要什么 | 状态 |
|---|---|---|---|
| B1 | linux × desktop（模式 2 产物） | 生成本机 desktop 产物并构建 | ✅ `build-verified`：本机 `cargo build --bin rustage` 与 `--features linux-native` 原生会话（[log-20261019-1 §5.7、§11](../log/log-20261019-1.md)）；生成物本身未链接，故不写 `certified` |
| B2 | linux × tablet / mobile | Linux target 的 profile rlib 与 cdylib 链接 | ✅ `build-verified`：固定依赖 2.8.10 以 `--no-default-features --features tablet|mobile --lib` 在 Linux x86_64 真实构建通过（[log-20261020-2 §6](../log/log-20261020-2.md)）；未运行 |
| B3 | linux × mini / embedded | Linux target 的 `no_std` 构建 | ✅ `build-verified`：2.8.10 修复了 `linux × mini` 的编译缺陷，mini 与 embedded 的 rlib 在 Linux x86_64 真实构建通过（[log-20261020-2 §6](../log/log-20261020-2.md)）；未运行 |
| B4 | macos × desktop | macOS 机器 | ✅ `build-verified`：desktop 构建通过；default 生成源码已由独立消费者 crate 链接运行 |
| B5 | macos × tablet / mobile | macOS 上的 profile 构建（运行仍需目标 SDK/设备） | ✅ `build-verified`：tablet/mobile rlib 在 aarch64-apple-darwin 构建通过；无设备运行 |
| B6 | macos × mini / embedded | 嵌入式工具链与板卡 | ✅ `build-verified`：两个 profile 的 rlib 真实构建通过；无板卡 |
| B7–B11 | windows × 五个 profile | Windows 机器（构建用交叉编译，运行仍需机器） | ✅ `build-verified` ×5：msvc 交叉构建 + tablet/mobile/mini/embedded rlib；运行待真机 |
| B12 | tablet/mobile 的**真设备**运行 | Android/iOS 真机 | ⏳ 待真机：路线图 T18.04 要求**模拟器与真设备分别记录**，不得互相顶替 |
| B13 | mini/embedded 的容量与链接审计 | 目标设备 + 容量门禁 | ⏳ 部分：实际 profile build 通过，链接审计基于真实 `cargo tree`；无目标设备 |
| B14 | 本地设备发现/选择/认证、打包导出与安装 | 每类设备 | ⏳ 部分：iOS/Android SDK 自动探测与真实设备发现已验证（xcrun simctl 11 台、adb 0 台）；macOS `codesign` 与 Linux `gpg` 均真实签名并验证；Windows `signtool` 与安装仍只到计划（[log-20261019-2 §5.8](../log/log-20261019-2.md)、[log-20261020-2 §6.3](../log/log-20261020-2.md)） |
| B15 | 五 profile 与静态核心一致性认证报告 | 需要以上全部证据 | ⏳ 部分：报告与一致性实测可用；15 格全部为构建级，但没有 profile 产物在对应设备/OS 上运行（[log-20261020-2 §6](../log/log-20261020-2.md)） |

## 3. 鸿蒙 2in1（不在"三宿主"内，单独跟踪）

| # | 项 | 需要什么 | 状态 | 证据 / 缺口 |
|---|---|---|---|---|
| C1 | DevEco 工程 + ArkTS 入口 + Rust GUI 小样 | HarmonyOS SDK / DevEco | ⛔ 阻塞 | 当前只有清单与资源模板（[issue-20261013 §5](../issues/issue-20261013.md)） |
| C2 | 2in1 设备上的窗口/绘制/事件循环承载 | 一台 2in1 设备 | ⛔ 阻塞 | 未通过承载小样前不得声明支持 |
| C3 | 签名包安装、桌面分层图标、启动页、应用身份 | 签名身份 + 设备 | ⛔ 阻塞 | [issue-20261013 §5、§6](../issues/issue-20261013.md) |

## 4. 性能与质量基线（T00.07 / T21）

| # | 项 | 需要什么 | 状态 | 缺口 |
|---|---|---|---|---|
| D1 | 16 个 `unmeasured` 场景的原始数据 | 固定硬件基线（本机即是参考机之一） | ⏳ 待真机 | 一个数据点都没有；`cold-start`、`workspace-switch` 这类还要在**每台宿主**上各测一次 |
| D2 | 目标侧性能与资源体积 | 目标设备/SDK | ⛔ 阻塞 | [benchmark.md](../plan/benchmark.md) 明确：宿主数字不得当目标数字 |
| D3 | 视觉基线在真实渲染下的确认 | 人 + 真实显示 | ⏳ 待真机 | `tests/baselines/icons/*.svg` 是后端渲染出的矢量，未经屏幕确认 |

## 5. 上游修好后必须用真机复核的条目

本地 HEAD 已经实现了 [`feedback-20261015.md`](../rw-feedback/feedback-20261015.md) 第二节的全部条目；
re-pin 已于 2026-10-15 完成（依赖升到 `58012c05`，见 [log-20261015-3](../log/log-20261015-3.md)），
因此下列项的"等 re-pin"前提已解除，剩下的只是**真机确认**：

| # | 项 | 真机上要确认什么 | 状态 |
|---|---|---|---|
| E1 | 系统文件对话框（`choose_path`/`open_path`/`save_path`） | 三桌面上真的弹出系统选择器并能取消；探针现在读 `capabilities().file_dialog`（状态后端 false、原生后端 true） | ⏳ 待真机（re-pin 已完成） |
| E2 | `quit()` 跨线程停止 | 三桌面上从 watcher 线程结束会话、进程干净退出：`--launch --exit-after-ms` 在原生会话上已实测 `LoopEnd::HostEnded` + 退出码 0（[log-20261015-3](../log/log-20261015-3.md)），macOS/Windows 待真机 | ⏳ 待真机（Linux 已验，macOS/Windows 待真机） |
| E3 | 掩码输入的复制 | 真机剪贴板里拿到的是掩码而不是明文（Linux 剪贴板已可测，Windows/macOS 需真机） | ⏳ 待真机 |
| E4 | 树的层级与就地改名 | 在真实窗口里展开/折叠、就地改名、滚动跟随 | ⏳ 待真机（还依赖 T04 的工作台） |
| E5 | watcher 的删除/重命名事件 | 在真实文件系统（含网络盘/挂载点）上触发删除与改名 | ⏳ 待真机 |
| E6 | 网络盘与挂载点边界 | NFS/SMB 上的行为 | ⏳ 待真机 |

## 6. 怎么补：每台机器上的最小步骤

```sh
# 任意宿主（桌面）：
cargo build                                  # 宿主构建证据（A20）
cargo test                                   # 宿主侧行为
cargo run -- --probe-host target/host        # 15 项探针 → 写回 host-probe.md 的该行
cargo run -- --launch                        # 窗口出现/接受输入 → A2（需人工观察）
cargo run --features linux-native -- --probe-host target/host   # Linux 原生 GTK 会话（A4）

# Windows（构建可先在 Linux 上交叉做，运行要在 Windows 上）：
cargo build --target x86_64-pc-windows-gnu   # A18
cargo clippy --target x86_64-pc-windows-gnu --all-targets

# Linux 上的 Wayland 会话（A5）：在 Wayland 桌面里重跑探针命令
```

**写回哪里**：宿主的探针结果 → [`host-probe.md`](../plan/host-probe.md)（该记录只允许**一台**宿主标
`probed`；换机器是把那一行换掉，不是新增一列）；profile/设备证据 →
[`support-matrix.md`](../plan/support-matrix.md) 的对应格；过程、原始输出与未覆盖项 → 当轮的
`docs/log/log-*.md`。**设备验收**始终是人工门禁（见 [traceability.md](../plan/traceability.md)）。

## 7. 别把这几样当成真机证据

1. **Wine**：能证明"产物能启动、跨平台代码路径能走通"（本项目已借此抓到三处只在 Windows 上可见的
   缺陷），不能证明该 OS 的运行时行为（[规则 6](../plan/support-matrix.md)、[log-20261015-2 §3](../log/log-20261015-2.md)）。
2. **状态后端（`PreviewOrStub`）**：它跑的是状态层，不合成窗口；`startup-session` 在那里的正确答案
   是 `not-verified`（[log-20261014-1 §0](../log/log-20261014-1.md)）。
3. **模拟器**：T18.04 要求模拟器与真设备**分别记录**，模拟器通过不等于设备通过。
4. **构建通过**：`build-verified` 不是 `certified`；"能编译"不能替"能运行"（[支持矩阵](../plan/support-matrix.md)）。

## 8. 当前的阻塞总览

| 缺什么 | 阻塞了哪些项 | 解锁需要 |
|---|---|---|
| 一台 macOS 机器 | A6、A9、A12（macOS 侧）、B4、B5 | 真机；Apple 账号另见 A21 |
| 一台 Windows 机器 | A7、A8、A10、A13、B7–B11 | 真机；构建一半已由 A18 覆盖 |
| 一台 HarmonyOS 2in1 设备 + 签名 | C1–C3 | 设备、SDK、签名身份 |
| Android NDK / iOS 工具链 | B2、B5、B12、D2 | 各目标 SDK |
| 嵌入式工具链与板卡 | B3、B6、B13 | 目标工具链与硬件 |
| 人在屏幕前 + 可切换缩放/多屏 | A2、A11、A15、A17、D3 | 一次人工验收 |
| 目标侧性能环境 | D2 | 目标设备/SDK |
