# 支持矩阵：三宿主 × 五 profile

> 数据源：[`support-matrix.json`](support-matrix.json)。本文件中的表格由
> [`tools/support-matrix/check-support-matrix.mjs`](../../tools/support-matrix/check-support-matrix.mjs)
> 逐行对照该文件校验，改一处而漏另一处会失败。
> 依据：[项目原则](principle.md) #2/#11/#12/#24/#25、[完整路线图](roadmap.md) T00.03。
> 需要真机/真设备的项汇总（含已验证标记）：[`docs/platform/verification.md`](../platform/verification.md)。
> 状态：**矩阵已建立；宿主构建/运行证据有三处——macOS（certified，另有 cocoa NativeInteractive 有界窗口会话）、
> Linux（certified，构建与有界 GTK 会话见 [log-20261019-1.md](../log/log-20261019-1.md) §2、§5.7）与 Windows（`x86_64-pc-windows-msvc`
> 真实交叉构建 + `x86_64-pc-windows-gnu` 既有证据，见 [log-20261015-2.md](../log/log-20261015-2.md) §2）。MS10 与后续复核之后，
> 三宿主的五个 profile 产物均取得真实构建证据；linux × tablet/mobile/mini/embedded 已于 2026-10-20 在 Linux 宿主补齐
> （2.8.10 修复了 linux × mini 的编译缺陷），见 [log-20261020-2.md](../log/log-20261020-2.md)。**

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
| macos | aarch64-apple-darwin | certified | cargo build --bin rustage 与 target/debug/rustage --version；x86_64-apple-darwin 交叉构建后在 Rosetta 下运行通过（MS10，见 docs/log/log-20261016-4.md §28）；--launch --exit-after-ms 1500 在 cocoa/NativeInteractive 下真实创建 1280x800 窗口并有界结束（--smoke 也创建了三个真实控件），见 docs/log/log-20261019-2.md §6.1 |
| windows | x86_64-pc-windows-msvc（真实构建）/ x86_64-pc-windows-gnu（既有构建证据） | build-verified | cargo build --target x86_64-pc-windows-msvc --bin rustage 通过（macOS 宿主 + rust-lld + xwin MSVC SDK/CRT，产出 48M PE32+ rustage.exe）；x86_64-pc-windows-gnu 的构建与 clippy 见 docs/log/log-20261015-2.md §2；运行证据仍缺（无 Windows 实机，Wine 不计为实机证据） |
| linux | x86_64-unknown-linux-gnu | certified | cargo build --bin rustage 与 target/debug/rustage --version 在 Linux x86_64、rustc 1.98.1 上通过；cargo build --features linux-native 亦通过，--launch --exit-after-ms 8000 在 X11 下映射出真实 1280x800 GTK 窗口（xwininfo），绘制回调填充 clear colour；--launch --smoke 画出真实按钮与文本框并经 XTest 指针事件触发回调；--smoke-worker 把点击经管道送进真实 --logic-worker 并观察到 running/idle；2026-10-20 在 2.8.10 上实跑 tools/gui-smoke/run.py --keys 与 --ime（fcitx5 preedit 逐帧可见、提交你好），--workbench-gui 随窗口尺寸 1280x800→1600x900→1000x700 重排；见 docs/log/log-20261019-1.md §2、§5.7 与 docs/log/log-20261020-2.md §3、§4 |

设计器宿主始终是桌面（原则 #2）；上表的"设计器宿主"指**设计器自身**能否在该 OS 上构建与运行，
与输出目标无关。

## profile 与产物模式

| profile | feature | 模式 1 | 模式 2 |
|---|---|---|---|
| desktop | desktop | unverified | build-verified |
| tablet | tablet | unverified | build-verified |
| mobile | mobile | unverified | build-verified |
| mini | mini | unsupported | build-verified |
| embedded | embedded | unsupported | build-verified |

- 模式 1（JSON + CSS 运行时加载）仅适用于 desktop/tablet/mobile；mini 与 embedded 必须使用
  模式 2（原则 #11）。这一格是**架构排除**，只会被明确决策改变，不会因为出现证据而变成可用。
- 模式 2 在全部 profile 上适用；三宿主的五个 profile 均已有真实构建证据（rlib）：`desktop` 在 macOS、
  Linux 与 Windows 宿主，`tablet`/`mobile`/`mini`/`embedded` 在 macOS、Linux 与 Windows 宿主。Linux 的四格
  由 2026-10-20 在 Linux x86_64 上以 2.8.10 补齐，见 `docs/log/log-20261020-2.md` §2。构建通过**不**证明
  产物能在该 profile 运行；没有把上述任何一格写成 `certified`。

## 矩阵（宿主 × profile，模式 2 产物）

| 宿主 \ profile | desktop | tablet | mobile | mini | embedded |
|---|---|---|---|---|---|
| macos | build-verified | build-verified | build-verified | build-verified | build-verified |
| windows | build-verified | build-verified | build-verified | build-verified | build-verified |
| linux | build-verified | build-verified | build-verified | build-verified | build-verified |

矩阵的证据范围（MS10 见 [log-20261016-4.md §28](../log/log-20261016-4.md)，Linux 补充见
[log-20261019-1.md](../log/log-20261019-1.md) 与 [log-20261020-2.md](../log/log-20261020-2.md)）：

- `macos × desktop`：`cargo build --bin rustage` 与 `target/debug/rustage --version` 通过；
  生成的 default 源码也已由 `tools/generated-project/run.py` 写进独立消费者 crate、真实链接并运行
  （`generated-ui-ok`）；该格仍是构建级证据，不是设备认证。
- `macos × mini`：`cargo build --manifest-path <rust-widgets>/Cargo.toml --no-default-features
  --features mini --lib` 通过（rlib 84,282,248 字节）；`--generate target:stripped` 产出 441 字节源码。
- `macos × embedded`：同一命令换成 `--features embedded` 通过（rlib 69,528,992 字节）。
- `linux × desktop`（2026-10-19）：`cargo build --bin rustage` 与 `target/debug/rustage
  --version` 在 Linux x86_64、rustc 1.98.1 上通过；`--logic-process` 的真实双进程走查也在同一
  宿主上通过（见 [log-20261019-1.md §2](../log/log-20261019-1.md)）。生成物本身尚未链接，
  因此这一格是构建级证据。
- `windows × desktop`（2026-10-19 第二次记录）：`cargo build --target x86_64-pc-windows-msvc --bin
  rustage` 在 macOS 宿主上用 rust-lld 与 xwin 提取的 MSVC SDK/CRT 通过，产出 PE32+ `rustage.exe`；
  尚未在 Windows 实机运行，故为构建级证据。见 [log-20261019-2.md](../log/log-20261019-2.md)。
- `macos × tablet` / `macos × mobile`（2026-10-19）：固定依赖以 `--no-default-features --features
  tablet|mobile --lib` 在 `aarch64-apple-darwin` 上真实构建通过；未运行。
- `windows × tablet/mobile/mini/embedded`（2026-10-19）：同一命令分别在 `x86_64-pc-windows-msvc`
  上用 rust-lld + xwin 的 MSVC SDK/CRT 真实构建通过；未运行。
- `tablet`/`mobile` 之前的两行“仍未验证”已不再成立：两个 profile 在 macOS、Linux 与 Windows 宿主上都有
  真实 profile 构建。Linux 的 `tablet`/`mobile`/`mini`/`embedded` 已在 2026-10-20 于 Linux x86_64 上以
  `--no-default-features --features <profile> --lib` 真实构建通过；`mini` 的 Linux 编译缺陷由上游 2.8.10
  修复（`locale::from_environment` 的 alloc-frugal 分支与 `compat::Vec` 导入）。证据见
  [log-20261020-2.md](../log/log-20261020-2.md) §2；没有把“编译到 rlib”写成“已运行”。

15 格现在全部为 `build-verified`。这是**事实而非保守**：三宿主的五 profile 都有真实构建记录，但没有
任何一格是 `certified`，因为 profile 产物（或宿主可执行文件）尚未在对应的目标设备/OS 上以该 profile
运行。矩阵的价值在于它是逐格可核对的声明点——补上证据时只改一格，而不是重写一段"基本支持"的描述。

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
