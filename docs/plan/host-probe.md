# 三桌面最小能力探针记录

> 数据源：[`host-probe.json`](host-probe.json)。本文件中的表格由
> [`tools/host-probe/check-host-probe.mjs`](../../tools/host-probe/check-host-probe.mjs)
> 逐行对照该文件校验，改一处而漏另一处会失败。
> 依据：[项目原则](principle.md) #12/#22/#24/#25、[完整路线图](roadmap.md) T01.08、
> [MS01 实施步骤](blue2.md) T01.08。
> 状态：**Linux 一组真实结果（本轮有两种构建形状：默认构建为状态后端，`--features linux-native`
> 为原生 GTK 会话）；macOS 与 Windows 标为 `blocked`（缺测试机），没有测量。**

## 这份记录回答什么

**一格 = 在某一桌面宿主上，某一组最小能力是否被真正测过、结果是什么。**
它不回答"设计器好不好用"，也不把"能编译"当成"能力可用"（原则 #12）。
"测过"与"结果通过"是两件事：一格的 `status` 说有没有测量，`outcome` 说测量到了什么。
本机（Linux）就有一格是 `probed` + `not-verified`，另一格是 `probed` + `gap`——
它们不是失败，也不是通过，而是**在这台机器上测得的真实事实**。

## 覆盖的组

路线图 T01.08 要求的最小集合是**启动、输入、进程、文件**四类；本记录有六组，因为宿主
实际具备的能力比这四类多两类（平台服务、工程作用域与权限），把它们藏起来会让记录
看起来比实现窄。**每个检查恰好属于一组**，由检查脚本对照探针源码里的能力名双向校验：
探针新增或改名一个检查而不更新本记录时，门禁会失败。

| 组 | 说明 | 覆盖的检查 |
|---|---|---|
| startup | 启动：会话起得来，环境说得清楚 | startup-session, host-directories |
| input | 输入：指针与键盘事件真的到达已挂载的控件 | input-events |
| process | 进程：后台任务可提交可取消，且只清理本会话拥有的进程树 | background-executor, process-trees |
| file | 文件：选择器（系统与自绘）、目录事件，以及选择结果是否指向真实文件 | native-file-dialog, chooser-frame, directory-watch |
| platform | 平台服务：剪贴板、URL 处理程序与交接、网络服务授权与脱敏 | clipboard, url-handler, url-handover, network-services |
| scope | 工程作用域与权限：信任、通知、关闭后不留悬挂服务 | project-trust, notifications, project-scope |

## 状态与结果词汇

| 状态 | 含义 |
|---|---|
| probed | 在本机真实跑过该组检查，结果与证据写在同一行的 outcome/evidence 里 |
| blocked | 没有可运行的测试机，因此没有测量；缺什么写在 missing 里 |
| unsupported | 该平台或该构建不提供此能力，且有明确决策；不能因为换机器而变成可用 |

| 结果 | 含义 |
|---|---|
| ready | 检查在本机跑通，行为与文档一致 |
| gap | 本机缺此能力，或行为与文档不符；原因在证据里 |
| not-verified | 本机无法产出此项证据（例如后端没有合成窗口） |

## 宿主与环境

| 宿主 | OS | 架构 | triple | 后端 | 本机 | 缺什么 |
|---|---|---|---|---|---|---|
| linux | linux | x86_64 | x86_64-unknown-linux-gnu | gtk (NativeInteractive)（以 `--features linux-native` 构建）；默认构建是 linux-state-backend (PreviewOrStub) | probed | — |
| macos | macos | aarch64 | aarch64-apple-darwin | — | blocked | 没有可用的 macOS 测试机。log-20261013-4 的 macOS 证据来自 T01.01 的自身启动（后端 cocoa、模式 NativeInteractive），不是本探针的 14 项检查 |
| windows | windows | x86_64 | x86_64-pc-windows-msvc | — | blocked | 没有可用的 Windows 测试机，本探针一次都没有在那里运行过。（本轮做了两件相邻的事，都不改变这一行：① cargo build --target x86_64-pc-windows-gnu 通过，产物在 Wine 9.0 下跑过 --probe-host，但那回答的是 Wine 的实现，按 support-matrix 规则 6 不计为实机证据；② 探针有 4 项检查（process-trees、project-trust、notifications、project-scope）需要一个 shell 作为受控程序，Windows 上没有 sh，因此那些格即使在真机上也需要先改造夹具。见 docs/log/log-20261015-2.md §3–§5） |

## 矩阵（宿主 × 组）

| 宿主 \ 组 | startup | input | process | file | platform | scope |
|---|---|---|---|---|---|---|
| linux | probed: ready | probed: ready | probed: ready | probed: gap | probed: not-verified | probed: ready |
| macos | blocked | blocked | blocked | blocked | blocked | blocked |
| windows | blocked | blocked | blocked | blocked | blocked | blocked |

`macos` 与 `windows` 的每一格都是 `blocked`，这是事实而不是保守：本仓库没有可用的
macOS 或 Windows 测试机，本探针一次都没有在那里运行过。表格行是"格"，不是"结论"——
补上证据时只改一格。

## 本机证据（`--probe-host` 的真实输出）

| 宿主 | 组 | 结果 | 证据 |
|---|---|---|---|
| linux | startup | ready | startup-session -> ready：以 `--features linux-native` 构建后，宿主协议把会话交给一个独立的 rustage 子进程（`--launch --log-level info --exit-after-ms 150`）；该子进程真的创建了窗口（window 6000285942072475648，1280x800）并进入 glib 主循环，到点按期限结束，父进程据此写出完整报告；同一台机器上用默认构建时本项为 not-verified（后端 linux-state-backend 报 PreviewOrStub，子进程以退出码 3 拒绝，写明窗口不会被合成）。host-directories -> ready：四个宿主目录逐个探测，config/cache/data 缺席而 temp 可用，并在临时根下逐个创建成功（两种构建相同） |
| linux | input | ready | input-events -> ready：控件内部按下并抬起 -> clicked(1)；按下后在控件外抬起 -> clicked 仍为 1 且 canceled(1)（证明真的做了命中测试）；按 Return -> clicked(2) |
| linux | process | ready | background-executor -> ready：提交在 8µs 返回而任务仍在跑，cancelling 可观察，配合的 job 被取消，无视 30ms 期限的 job 被记为 abandoned，关闭后拒绝新工作；process-trees -> ready：本会话启动的进程与其后代被整组清理，同名但本会话之外启动的进程存活 |
| linux | file | gap | native-file-dialog -> gap：固定修订在这三个桌面后端上不创建文件对话框控件（create_file_dialog 返回 0），因此拿不到系统自己的选择器；chooser-frame -> ready：挂载、Return 选中真实路径、Escape 取消、无选择时返回被拒；directory-watch -> ready：目录内一次写入产生 1 个事件，监听不存在的目录被明确拒绝 |
| linux | platform | not-verified | clipboard -> ready：写回一个 35 字符的非 ASCII 串并读回成功（由状态后端回答）；url-handler -> ready：xdg-open 在 PATH 上，选项形状与无 scheme 的目标在启动前被拒；url-handover -> not-verified：探针不得打开用户的浏览器，交接只在宿主自己的测试里对受控处理程序验证；network-services -> ready：声明的回环服务在许可下真的收到 4 字节请求并被应答，策略未声明的服务被拒，记录过期的凭据在发出请求前被诊断并给出下一步，凭据在许可渲染与日志行里都不出现 |
| linux | scope | ready | project-trust -> ready：未信任工程被拒且没有写出标记文件，信任后仍需授权范围，撤销授权后再次被拒，产物里带完整授权记录；notifications -> ready：真实失败的任务/进程、缺失程序与真实权限拒绝各自成为带原因与下一步的通知，重复只计数；project-scope -> ready：两个工程各自启动一个进程，关闭其一后它的进程消失、另一个存活、关闭的作用域句柄拒绝新工作，且关闭顺序被如实回报 |

## 规则

1. **缺测试机就写 `blocked`，并写清缺什么。** 不把未测量写成 `unsupported`（那是"架构排除"
   的判断），也不写成 `probed`（那是"测过了"的声明）。`blocked` 的格必须有原因。
2. **`probed` 只允许出现在本记录产生的那台机器上。** 检查脚本要求恰有一台宿主 `probedOn`，
   并且 `probed` 的格必须写 `outcome` 与证据；不允许把别处的结论抄进来。
3. **`outcome` 用探针自己的三级词汇**（`ready` / `gap` / `not-verified`），与 `--probe-host` 的
   渲染和 `host-readiness.json` 完全一致，不另立一套说法。
4. **能力名来自探针源码，不来自本文件。** 检查脚本从 `src/host/readiness.rs` 的
   `Capability::name` 读取全部检查名，要求它们在组里**恰好各出现一次**。
5. **两组记录回答不同问题。** [`support-matrix.md`](support-matrix.md) 回答"在某一宿主上为某一
   输出 profile 建立构建或运行证据的状态"；本记录回答"宿主的平台能力在这台机器上被测量到的样子"。
   两者都可能因为一台新机器而多一行，但不会互相替代。

## 跑一次要什么

```sh
cargo run -- --probe-host target/host    # 写出 target/host/host-readiness.json；有检查未就绪时退出码 4

# Linux 桌面宿主：默认构建的后端是 linux-state-backend（状态层，不合成窗口），因此 startup-session
# 只能报 not-verified；转发上游 `gtk-native` 才会得到真实窗口（后端 gtk，模式 NativeInteractive）：
cargo run --features linux-native -- --probe-host target/host
```

**为什么 Linux 要写两种构建**：同一台机器、同一条命令，`startup-session` 的答案取决于构建里有没有
原生后端（上游用 `gtk-native` 这个 feature 选择它），所以"Linux 一格"不足以表达这台机器上被测到的
事实。两行的证据都写在上面：默认构建 `not-verified`（窗口不会被合成），`--features linux-native`
为 `ready`（子进程真的创建了窗口）。`--probe-host` 现在就绪与否都要写出报告与产物，因此两种构建
都能留下可复核的 JSON。

在三台机器上跑同一条命令即可补齐本记录的对应列；把结果按同一组粒度抄进
`host-probe.json`（`status` 改成 `probed`、`outcome` 抄检查的状态、`evidence` 抄检查的证据），
再运行一次检查脚本，它会把任何落下的表行指出来。
