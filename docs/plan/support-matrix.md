# 支持矩阵：三宿主 × 五 profile

> 数据源：[`support-matrix.json`](support-matrix.json)。本文件中的表格由
> [`tools/support-matrix/check-support-matrix.mjs`](../../tools/support-matrix/check-support-matrix.mjs)
> 逐行对照该文件校验，改一处而漏另一处会失败。
> 依据：[项目原则](principle.md) #2/#11/#12/#24/#25、[完整路线图](roadmap.md) T00.03。
> 需要真机/真设备的项汇总（含已验证标记）：[`docs/platform/verification.md`](../platform/verification.md)。
> 状态：**矩阵已随 T00.01 升级重估到 3.2.1（`be69fa2b`，2026-11-02）：macOS 在 3.2.1 上有 `cargo test --bin`
> 多域通过证据（[log-20261101-2.md](../log/log-20261101-2.md)），Linux 在 2026-11-02 有 `cargo test probe::`
> 19/19 通过证据（feedback-20261101.md 决策记录），两宿主均为 `build-verified`；macOS 的 GUI 有界会话与
> Windows xwin 交叉构建未在 3.2.1 复跑，相应格子按“不拿旧修订证据冒充新修订”降为 `unverified` 并保留
> 3.1.0 历史证据引用（[log-20261029-11.md](../log/archived/log-20261029-11.md)）。
> 2.8.11 的 embedded E0599 已由 2.8.12 修复并在 3.1.0 复验，见
> [feedback-20261020.md](../rw-feedback/feedback-20261020.md) §0.6。**

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

## 容量预算与 desktop 独有能力

下表是库公布的三档预算（路线图 §1.6.1），不是宿主的实测数字；宿主遇到容量溢出
时必须阻断生成并说明原因（原则 #13），不得静默丢弃控件。

| 档位 | 控件 | 纹理 | 字形缓存 | 事件队列 |
|---|---|---|---|---|
| Device | 4096 | 4096 | 2 MiB | 256 |
| Surface | 512 | 2048 | 1 MiB | 128 |
| Minimal | 64 | 1024 | 256 KiB | 64 |

**仅 desktop 提供：** 图表（chart）、打印（print）、PDF、彩色 emoji。tablet/mobile 可承载
矢量字体与图标但不能据此声明图表/打印/PDF/彩色 emoji；mini/embedded 是 sized profile，
不承载这些能力。


## 宿主

| 宿主 | triple | 设计器宿主 | 理由 |
|---|---|---|---|
| macos | aarch64-apple-darwin | build-verified | 3.2.1（be69fa2b）上 cargo test --bin rustage 多域测试在 macOS aarch64 构建并通过（rust_widgets 以 desktop 依赖 features 构建），见 docs/log/log-20261101-2.md；3.1.0（d44ca0e9）的 --workbench-gui 真实 1280x800 窗口会话证据保留于 docs/log/archived/log-20261029-11.md，但 3.2.1 未复跑 GUI 有界会话，故由 certified 降为 build-verified |
| windows | x86_64-pc-windows-msvc（真实构建）/ x86_64-pc-windows-gnu（既有构建证据） | unverified | 3.1.0（d44ca0e9）曾在 macOS 宿主用 rust-lld + xwin 交叉构建 --bin rustage（69 MB PE32+），见 docs/log/archived/log-20261029-11.md；3.2.1（be69fa2b）未复跑交叉构建，按“不拿旧修订证据冒充新修订”降为 unverified |
| linux | x86_64-unknown-linux-gnu | build-verified | 2026-11-02 在 Linux x86_64 上 cargo test probe:: 通过（probe 契约 19/19，五域探针全 Verified；rust_widgets 以 3.2.1 desktop 依赖 features 完整构建），见 docs/rw-feedback/feedback-20261101.md 决策记录；产品 GUI 运行需 linux-native（GTK）特性，未运行，故为 build-verified |

设计器宿主始终是桌面（原则 #2）；上表的"设计器宿主"指**设计器自身**能否在该 OS 上构建与运行，
与输出目标无关。

## profile 与产物模式

| profile | feature | 模式 1 | 模式 2 |
|---|---|---|---|
| desktop | desktop | unverified | build-verified |
| tablet | tablet | unverified | unverified |
| mobile | mobile | unverified | unverified |
| mini | mini | unsupported | unverified |
| embedded | embedded | unsupported | unverified |

- 模式 1（JSON + CSS 运行时加载）仅适用于 desktop/tablet/mobile；mini 与 embedded 必须使用
  模式 2（原则 #11）。这一格是**架构排除**，只会被明确决策改变，不会因为出现证据而变成可用。
- 模式 2 在 3.2.1 上有 desktop 产物证据（macOS cargo test --bin、Linux cargo test probe::）；
  tablet/mobile/mini/embedded 在 3.1.0 上曾有 macOS 与 Windows 交叉构建证据，3.2.1 未复跑，
  相应格子按矩阵原则降为 `unverified`。构建通过**不**证明产物能运行；没有把任何一格写成 `certified`。

## 矩阵（宿主 × profile，模式 2 产物）

| 宿主 \ profile | desktop | tablet | mobile | mini | embedded |
|---|---|---|---|---|---|
| macos | build-verified | unverified | unverified | unverified | unverified |
| windows | unverified | unverified | unverified | unverified | unverified |
| linux | build-verified | unverified | unverified | unverified | unverified |

矩阵的证据范围（3.2.1 的新证据见 [log-20261101-2.md](../log/log-20261101-2.md) 与
[feedback-20261101.md](../rw-feedback/feedback-20261101.md) 决策记录；3.1.0 的历史证据见
[log-20261029-11.md](../log/archived/log-20261029-11.md)；2.8.12 的记录见
[log-20261020-4.md](../log/log-20261020-4.md)）：

- `macos` 设计器宿主与 `macos × desktop`（3.2.1）：`cargo test --bin rustage` 在 macOS aarch64 上构建并通过多域测试
  （rust_widgets 以 desktop 依赖 features 构建）；GUI 有界会话与 profile 专属 `--lib` 构建未在 3.2.1 复跑。
- `linux` 设计器宿主与 `linux × desktop`（3.2.1）：`cargo test probe::` 于 2026-11-02 在 Linux x86_64 通过
  （probe 契约 19/19，五域探针全 Verified）；产品 GUI 运行需 `linux-native`（GTK），未运行。
- `macos × tablet/mobile/mini/embedded`（3.1.0 历史证据）：`cargo build --manifest-path <rust-widgets>/Cargo.toml
  --no-default-features --features <profile> --lib` 曾在 macOS aarch64、rustc 1.98.1 上真实通过；
  3.2.1 未复跑，格子降为 `unverified`。
- `windows × desktop/tablet/mobile/mini/embedded`（3.1.0 历史证据）：曾以 rust-lld + xwin 交叉构建通过，
  `--bin rustage` 产出 69 MB PE32+；3.2.1 未复跑，格子降为 `unverified`。
- `linux × tablet/mobile/mini/embedded`：3.1.0 与 3.2.1 均未复跑；2.8.11/2.8.12 的旧证据不作为新修订证据。

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
