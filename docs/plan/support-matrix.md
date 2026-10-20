# 支持矩阵：三宿主 × 五 profile

> 数据源：[`support-matrix.json`](support-matrix.json)。本文件中的表格由
> [`tools/support-matrix/check-support-matrix.mjs`](../../tools/support-matrix/check-support-matrix.mjs)
> 逐行对照该文件校验，改一处而漏另一处会失败。
> 依据：[项目原则](principle.md) #2/#11/#12/#24/#25、[完整路线图](roadmap.md) T00.03。
> 需要真机/真设备的项汇总（含已验证标记）：[`docs/platform/verification.md`](../platform/verification.md)。
> 状态：**2.8.11 重钉后，只有 Linux 宿主重新取得了五个 profile 的构建记录；四格通过，embedded 因上游缺陷失败。
> macOS 与 Windows 的 2.8.11 构建未在本机复跑，因此按原则 #25 退回 `unverified`（2.8.10 的旧证据保留在日志里，
> 不作为 2.8.11 证据）。Linux × embedded 与所有 embedded 格因 2.8.11 的 E0599 编译失败标为 `unsupported`，
> 缺陷见 [feedback-20261020.md](../rw-feedback/feedback-20261020.md) 与
> [log-20261020-3.md](../log/log-20261020-3.md)。**

## 这份矩阵回答什么

**一格 = 在某一宿主上，为某一输出 profile 建立构建或运行证据的状态。**
它不回答"设计器好不好用"，也不把库的 feature 名称当作可用性证明（原则 #12）。
每格必须取下列四个级别之一，且**除 `unverified` 外的每一格都要写明理由**。

## 状态级别

| 级别 | 含义 |
|---|---|
| `unsupported` | 项目架构排除，或固定依赖不提供；没有明确决策不得打开 |
| `unverified` | 本仓库没有构建或运行的记录 |
| `build-verified` | 有记录的构建通过；尚未运行 |
| `certified` | 在所指 OS/triple/设备上有记录的构建与运行 |

`build-verified` 是路线图"未验证/不支持/认证"三级之外的第四级，理由是：**构建通过不等于运行
通过**，把它并入 `certified` 会虚报，并入 `unverified` 会丢掉真实证据。四级都要在证据审查时
写出命令与环境。

## 宿主

| 宿主 | triple | 设计器宿主 | 理由 |
|---|---|---|---|
| macos | aarch64-apple-darwin | unverified | 2.8.10 上有 cargo build --bin rustage 与有界 cocoa 会话（docs/log/log-20261019-2.md §6.1）；2.8.11 未在本机复跑，宿主格与 profile 格都不冒充新修订证据 |
| windows | x86_64-pc-windows-msvc（真实构建）/ x86_64-pc-windows-gnu（既有构建证据） | unverified | 2.8.10 上有 msvc 交叉构建（macOS 宿主 + rust-lld + xwin，docs/log/log-20261019-2.md）；2.8.11 未在本机复跑，宿主格与 profile 格都不冒充新修订证据；无 Windows 实机，Wine 不计为实机证据 |
| linux | x86_64-unknown-linux-gnu | certified | 2.8.11 上 cargo build --bin rustage、--features linux-native、五个 profile 的 --lib 构建均通过；--launch --smoke 与 tools/gui-smoke/run.py --keys/--ime 使用真实 GTK/X11 与 fcitx5；--workbench-gui 在 2.8.11 上用 wmctrl 缩放后无需输入即显示新布局；见 docs/log/log-20261020-3.md |

设计器宿主始终是桌面（原则 #2）；上表的"设计器宿主"指**设计器自身**能否在该 OS 上构建与运行，
与输出目标无关。

## profile 与产物模式

| profile | feature | 模式 1 | 模式 2 |
|---|---|---|---|
| desktop | desktop | unverified | build-verified |
| tablet | tablet | unverified | build-verified |
| mobile | mobile | unverified | build-verified |
| mini | mini | unsupported | build-verified |
| embedded | embedded | unsupported | unsupported |

- 模式 1（JSON + CSS 运行时加载）仅适用于 desktop/tablet/mobile；mini 与 embedded 必须使用
  模式 2（原则 #11）。这一格是**架构排除**，只会被明确决策改变，不会因为出现证据而变成可用。
- 模式 2 在 desktop/tablet/mobile/mini 上仍有 2.8.11 的 Linux 构建证据；macOS/Windows 在 2.8.11 上未复跑。
  `embedded` 的 2.8.11 构建在四条构造器上失败（E0599），因此模式 2 暂标 `unsupported`；这是固定依赖的缺陷，
  不是架构排除，上游修复后应重新打开。构建通过**不**证明产物能运行；没有把任何一格写成 `certified`。

## 矩阵（宿主 × profile，模式 2 产物）

| 宿主 \ profile | desktop | tablet | mobile | mini | embedded |
|---|---|---|---|---|---|
| macos | unverified | unverified | unverified | unverified | unsupported |
| windows | unverified | unverified | unverified | unverified | unsupported |
| linux | build-verified | build-verified | build-verified | build-verified | unsupported |

矩阵的证据范围（2.8.11 的本轮记录见 [log-20261020-3.md](../log/log-20261020-3.md)；2.8.10 的旧记录见
[log-20261020-2.md](../log/log-20261020-2.md)）：

- `linux × desktop/tablet/mobile/mini`（2.8.11）：`cargo build --manifest-path <rust-widgets>/Cargo.toml
  --no-default-features --features <profile> --lib` 在 Linux x86_64、rustc 1.98.1 上真实通过。
- `linux × embedded`（2.8.11）：同一命令失败，错误是 `WidgetKind::TabWidget/Splitter/Grid/Dialog`
  不存在；四个新构造器用 `#[cfg(not(alloc_frugal))]` 门控，而变体由 `widgets_unstripped` 门控。
  这是固定依赖的缺陷，已记入上游反馈。
- `macos × *` 与 `windows × *`：2.8.11 未在本机复跑；表中退回 `unverified`，旧证据不冒充新修订。
- `macos × desktop` 的设计器宿主 `certified` 仍描述 2.8.10 的窗口会话；它不覆盖 profile 格。

## 规则

1. **未验证不是不支持，也不是支持。** `unverified` 只表示没有记录；既不能据它宣称可用，
   也不能当成已知缺陷。
2. **认证需要构建与运行两条证据**，且命令、环境、OS/triple、设备与日期必须可复核（原则 #24）。
   模拟、静态清单或宿主预览不能替代目标构建。
3. **能力以实际契约为准。** 每格的可用性应由目标 profile 的能力查询回答；本仓库的
   `platform::profile` 探针读回当前编译 profile（`desktop`），作为矩阵 `desktop` 行的锚点。
4. **不掩盖缺设备。** 缺少 SDK、测试机或签名环境时，该格保持 `unverified` 并写明缺什么，
   不得改为 `certified`。
5. **单列范围外平台。** HarmonyOS 2in1 的打包资源不属于"三宿主"，其验收项单独跟踪于
   [`docs/issues/issue-20261013.md`](../issues/issue-20261013.md) 第 5、6 节，本矩阵不为它
   增设宿主行。
6. **Wine 不是实机替代。** Wine 上跑 Windows 产物能证明的东西有三样：产物真的能启动、跨平台代码
   路径在另一套 OS 接口下能走通、以及缺陷会被提前暴露（本项目就是这样抓到三处只在 Windows 上可见的
   问题，见 [log-20261015-2.md](../log/log-20261015-2.md)）。它不能证明的是：该产物在其所指的 OS 上
   如何运行——Wine 的剪贴板接的是 X11、IME/DPI/Job Object/系统对话框都是另一套实现，而它"返回成功
   却没做"的桩函数正是原则 #22 要防的形状。因此 Wine 下的结果**既不进本矩阵的 `certified`**，
   **也不进 [`host-probe.md`](host-probe.md) 的 `probed` 行**（那条记录要求恰好一台被测量的机器）：
   它写进开发记录，并注明版本（本轮是 Wine 9.0、`WINEARCH=win64`）。判定一句"能不能用 Wine 顶替"
   的准则是**先分清要证明的是构建还是运行**：构建可以用交叉编译真正地证明，运行不能。
