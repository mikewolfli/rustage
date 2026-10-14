# 公共契约：对象、变更集、权限、版本、诊断与源映射

> 数据源：[`contracts.json`](contracts.json)。本文件的表格由
> [`tools/contracts/check-contracts.mjs`](../../tools/contracts/check-contracts.mjs) 逐行对照该
> 文件校验；同一脚本还解析白皮书 §12.5 的权限表、核对 `src/contracts/` 的写入入口，并确认每个
> 不变量点名的用例确实存在于它声明的文件里。改一处而漏另一处会失败。
> 依据：[项目原则](principle.md) #6/#22/#23/#25/#59、[完整路线图](roadmap.md) T00.05、
> [白皮书](whitepaper.md) §12.3/§12.5/§17.6、[决策登记](decisions.md)。
> 状态：**契约与不变量已定义并由测试强制；设计器本身尚未实现，契约的消费方是 T02.01、T03.01、
> T12.04、T15.03 等后续任务。**

## 这份文件回答什么

T00.05 的最低验收是「跨视图/服务没有第二份可写模型」。这不是一句设计口号，而是两个可检查的
问题：**谁能写、以及写进去的东西能不能被追溯**。本文件分三层回答：

1. **契约区域**：文档模型、身份、版本、变更、权限、诊断、源映射各自关心什么、实现在哪个文件。
2. **唯一写入入口**：`src/contracts/` 里所有会改动状态的公开方法被逐条声明；多出一个写入入口而
   不更新数据文件，检查就会失败。
3. **不变量**：每条不变量都点明"由哪个文件强制、由哪个用例证明、证据强度是什么"。证据强度是
   刻意写出来的：**结构强制**（类型不允许该类调用）强于**测试**，而测试只证明它写过的那条路径。

本文件不回答"实现是否已经完成"。契约的消费方（文档格式、带文本缓冲的工程存储、属性 schema、
生成器与源映射）都还没有落地；本文件只保证它们落地时只有一套形状可用。

## 不在本任务范围内

- **文档格式（T02.01）**：本文件不定字段名、扩展名与磁盘布局，只定内存中的契约。格式版本字段
  的存在性来自决策 `Q6`，具体 schema 归 T02.01。
- **文本缓冲与有效模型版本（T03.01）**：`DocumentStore` 目前只持有结构化模型，不含文本镜像。
  「无效文本保留、最后有效模型仍可预览」属于 T03.01。
- **属性 schema 与取值范围（T12.04）**：`PropertyValue` 只区分值族，不校验范围；范围表只能有一份，
  归 schema 拥有。
- **生成器与源映射写入（T15.02/T15.03）**：本文件定义映射的形状与过期判定，不定义生成算法。
- **取消与外部服务失效**：变更集是同步的、原子的，因此没有"取消到一半"的状态；任务取消、构建
  失败与服务恢复属于 T01.03/T16.02，本文件不声称覆盖。

## 契约区域

| 契约 | 关注点 | 模块 | 依据 |
| --- | --- | --- | --- |
| C-IDENTITY | 稳定节点/页面身份，永不复用 | [`src/contracts/identity.rs`](../../src/contracts/identity.rs) | principle #6（单一事实来源） |
| C-VERSION | 修订、版本戳与过期词汇 | [`src/contracts/revision.rs`](../../src/contracts/revision.rs) | principle #22（过期结果不得冒充当前） |
| C-MODEL | 页面、节点与类型化属性值 | [`src/contracts/model.rs`](../../src/contracts/model.rs) | principle #59（不得出现漂移的平行实现） |
| C-CHANGE | 变更集与其拒绝原因 | [`src/contracts/change.rs`](../../src/contracts/change.rs) | principle #23（变更可撤销） |
| C-PERMISSION | 能力、授权、拒绝与撤销 | [`src/contracts/permission.rs`](../../src/contracts/permission.rs) | principle #22（拒绝必须是准确结果） |
| C-DIAGNOSTIC | 问题、定位与过期标记 | [`src/contracts/diagnostics.rs`](../../src/contracts/diagnostics.rs) | principle #25（失败显式化） |
| C-SOURCEMAP | 节点与生成源码区间 | [`src/contracts/source_map.rs`](../../src/contracts/source_map.rs) | principle #24（结论需可复核证据） |
| C-STORE | 唯一可写模型与只读视图/快照 | [`src/contracts/store.rs`](../../src/contracts/store.rs) | principle #6（跨视图/服务没有第二份可写模型） |
| C-PROJECT | 工程文档、根相对路径引用与格式版本策略 | [`src/project/format.rs`](../../src/project/format.rs) | principle #22（拒绝必须是准确结果） |

## 权限词汇

| 授权 | 含义 |
| --- | --- |
| agent-autonomous | agent 可自主：可逆、无副作用、由变更集记录 |
| needs-review | 需评审通过：影响布局与生成代码结构 |
| needs-confirmation | 需人确认：没有生成器或门禁保护 |
| human-only | 仅人可触发：不存在 agent 路径，批准也不产生路径 |
| agent-read-only | agent 只读：生成物由生成器重写，手改会被覆盖 |
| authorized-when-trusted | 工程已信任且执行范围获授权后 agent 可触发 |

## 拒绝原因

| 拒绝原因 | 含义 |
| --- | --- |
| untrusted-project | 工程未信任：不运行工程代码 |
| needs-review | 等待人工评审该变更集 |
| needs-confirmation | 等待人工确认该变更 |
| human-only | 该操作只有人的路径 |
| generated-code-read-only | 生成物只读，改动应回到设计模型 |
| scope-not-authorized | 执行范围尚未授权 |
| service-not-configured | 服务不在工程网络策略里：先声明该服务并授权，再发起请求 |

## 操作类别与默认权限（白皮书 §12.5）

| 操作类别 | 机器名 | 默认权限 | 原文写法 | 理由 | 拒绝时 | 用例 |
| --- | --- | --- | --- | --- | --- | --- |
| 改画布上的控件属性 | `edit-widget-property` | `agent-autonomous` | **agent 可自主**（M1/M2 下） | 可逆、无副作用、有变更集记录 | — | `an_agent_may_change_a_widget_property_without_asking` |
| 增删控件节点 | `add-or-remove-node` | `needs-review` | **需评审通过** | 影响布局与生成代码结构 | `needs-review` | `an_agent_proposal_needing_review_is_refused_until_a_human_approves_it` |
| 修改生成的 Rust 代码 | `edit-generated-code` | `agent-read-only` | **agent 只读**（§12.6 Q7） | 会被下次生成覆盖 | `generated-code-read-only` | `an_agent_may_never_write_generated_output_even_with_approval` |
| 修改手写区（非生成区） | `edit-handwritten-code` | `needs-confirmation` | **需人确认** | 无生成器保护 | `needs-confirmation` | `an_agent_proposal_needing_review_is_refused_until_a_human_approves_it` |
| 落盘写文件 | `write-file` | `human-only` | **仅人可触发** | A-2 | `human-only` | `an_approval_never_opens_a_human_only_path` |
| 运行 `cargo check` | `run-cargo-check` | `authorized-when-trusted` | **工程已信任且执行范围获授权后 agent 可触发** | 可运行 build script/过程宏或访问网络，不是只读；须经宿主任务/权限门禁 | `untrusted-project` | `running_cargo_needs_trust_first_and_the_scope_afterwards` |
| 新增依赖 / 改 `Cargo.toml` | `change-dependencies` | `needs-confirmation` | **必须人确认** | 影响全部 profile 与门禁 | `needs-confirmation` | `the_default_table_matches_the_whitepaper_rows` |
| 对工程网络策略里的服务发起请求 | `request-network-service` | `authorized-when-trusted` | **工程已信任且服务范围获授权后 agent 可触发** | 会外发数据并可能携带凭据；只允许到达工程策略里声明并授权的服务，不暗自联网 | `untrusted-project` | `a_network_request_needs_the_service_declared_and_authorized` |

## 唯一写入入口

| 文件 | 种类 | 写入方法 | 理由 |
| --- | --- | --- | --- |
| [`src/contracts/store.rs`](../../src/contracts/store.rs) | public | `allocate_id`, `apply`, `gate_mut` | apply 是唯一改动节点的入口；gate_mut 改的是权限状态；allocate_id 只推进 id 序列，不改文档（见 C-STORE 的说明） |
| [`src/contracts/model.rs`](../../src/contracts/model.rs) | crate-internal | `remove_property`, `set_children`, `set_parent`, `set_property` | Node 的写入方法是 pub(crate)，只有 contracts::store 的 apply 路径可以调；视图与快照拿不到 &mut |

## 不变量与强制位置

| 编号 | 不变量 | 契约 | 强制位置 | 用例 | 证据 |
| --- | --- | --- | --- | --- | --- |
| INV-SINGLE-WRITER | 文档只有一份可写模型：跨视图/服务不存在第二份可写副本 | `C-STORE` | [`src/contracts/store.rs`](../../src/contracts/store.rs) | `a_document_starts_at_the_initial_revision_with_its_root` | 结构 + 测试 |
| INV-ATOMIC-CHANGE | 一个变更集要么全部生效并把版本推进一次，要么一条都不生效且版本不动 | `C-CHANGE` | [`src/contracts/store.rs`](../../src/contracts/store.rs) | `a_rejected_change_set_leaves_the_operations_before_it_unapplied` | 测试 |
| INV-INVERTIBLE | 每个操作自带被替换的状态，因此逆操作可推导，不需要另存一份逆操作 | `C-CHANGE` | [`src/contracts/change.rs`](../../src/contracts/change.rs) | `structural_operations_invert_to_the_exact_opposite_position` | 测试 |
| INV-STALE-BASE | 针对旧版本准备的变更集被拒绝，不会被隐式合并到新状态上 | `C-VERSION` | [`src/contracts/store.rs`](../../src/contracts/store.rs) | `a_change_set_against_an_older_revision_is_refused_and_changes_nothing` | 测试 |
| INV-IDENTITY-STABLE | 节点身份只增不复用：被移除的身份在新变更集里被拒绝，而不是复活同一个节点 | `C-IDENTITY` | [`src/contracts/store.rs`](../../src/contracts/store.rs) | `a_removed_identity_is_never_reused` | 测试 |
| INV-DEFAULT-DENY | 权限默认拒绝：未信任工程不执行代码，批准无法打开 human-only 与生成物只读 | `C-PERMISSION` | [`src/contracts/permission.rs`](../../src/contracts/permission.rs) | `withdrawing_trust_drops_the_grants_it_was_given_with` | 测试 + 交叉校验 |
| INV-DENIAL-EXPLAINED | 拒绝必须是显式且带原因的，不能表现为空成功 | `C-PERMISSION` | [`src/contracts/permission.rs`](../../src/contracts/permission.rs) | `every_denial_explains_what_would_have_to_change` | 测试 |
| INV-STALE-LABELED | 过期结果被标记为过期，不被当作当前结果展示 | `C-VERSION` | [`src/contracts/diagnostics.rs`](../../src/contracts/diagnostics.rs) | `a_stale_set_offers_no_current_entries_instead_of_old_ones` | 测试 |
| INV-DIAGNOSTIC-LOCATABLE | 诊断在缺少节点映射时仍保留源码定位 | `C-DIAGNOSTIC` | [`src/contracts/diagnostics.rs`](../../src/contracts/diagnostics.rs) | `a_node_diagnostic_keeps_its_source_position` | 测试 |
| INV-MOST-SPECIFIC-MAPPING | 嵌套区间取最具体的节点，未映射的位置返回空而不是猜测 | `C-SOURCEMAP` | [`src/contracts/source_map.rs`](../../src/contracts/source_map.rs) | `the_most_specific_span_wins_when_spans_nest` | 测试 |
| INV-EFFECT-DECIDED | 每个外部效果类别要么由决策表的一行裁定，要么显式说明它没有决定 | `C-PERMISSION` | [`src/contracts/permission.rs`](../../src/contracts/permission.rs) | `every_external_effect_class_is_decided_or_explained` | 测试 |
| INV-VERSION-UNKNOWN-NOT-WRITTEN | 本构建不认识的格式版本只读打开：不解释任何成员，也没有任何写回路径 | `C-PROJECT` | [`src/project/format.rs`](../../src/project/format.rs) | `a_newer_format_version_refuses_to_write_the_project` | 结构 + 测试 |
| INV-NO-FIELD-LOSS | 本构建不认识的成员被原样保留并报告，保存时写回原位，不静默丢弃 | `C-PROJECT` | [`src/project/format.rs`](../../src/project/format.rs) | `unknown_members_survive_a_round_trip_and_are_reported` | 测试 |
| INV-NO-HALF-PROJECT | 创建只在暂存目录里发生，最后一步用一次改名提交；失败或取消移除暂存目录，原目录不被触碰 | `C-PROJECT` | [`src/project/create.rs`](../../src/project/create.rs) | `a_failure_or_a_cancel_leaves_the_directory_as_it_was` | 结构 + 测试 |
| INV-NO-OVERWRITE | 已存在且含内容的目录在任何写入发生之前就被拒绝，本构建绝不覆盖已有的工程目录 | `C-PROJECT` | [`src/project/create.rs`](../../src/project/create.rs) | `a_failure_does_not_overwrite_a_directory_that_has_something_in_it` | 测试 |
| INV-CREATED-IS-READABLE | 创建出来的工程在提交前先被本构建自己的读码器读回：写出与能读回是同一件事 | `C-PROJECT` | [`src/project/create.rs`](../../src/project/create.rs) | `a_project_that_is_created_is_a_project_this_build_reads_back` | 测试 |
| INV-OPEN-DECIDES-NOTHING | 打开工程只读入文档并给出将被提出的问题：不记录决定、不授权、不运行任何程序 | `C-PROJECT` | [`src/project/session.rs`](../../src/project/session.rs) | `opening_reads_the_project_and_decides_nothing` | 测试 |
| INV-RECENT-IS-A-PLACE | 最近项只记“哪里”：用它打开等同于手打路径，同样不信任、不授权、不运行 | `C-PROJECT` | [`src/project/session.rs`](../../src/project/session.rs) | `a_recent_entry_opens_the_project_without_trusting_or_running_anything` | 结构 + 测试 |
| INV-PROMPT-IS-CURRENT | 信任提问在回答前被重新读取；目录在提问之后变了，答案不生效并撤销信任 | `C-PROJECT` | [`src/project/session.rs`](../../src/project/session.rs) | `a_directory_that_changed_while_the_question_was_on_screen_is_not_trusted` | 测试 |
| INV-MIGRATION-IS-VERSIONED | 格式变更附带带版本的迁移步骤；能到达本构建版本的旧文档被读取并在写回时先保留原文件，到不了的版本明确拒绝 | `C-PROJECT` | [`src/project/format.rs`](../../src/project/format.rs) | `an_older_version_is_migrated_by_versioned_steps` | 测试 |
| INV-FILE-OP-PREVIEWS | 文件操作先出计划：计划只读工程、写出要做的每一步，不写盘；有设计引用指向该文件时阻断并列出引用，而不是静默丢弃引用 | `C-PROJECT` | [`src/project/files.rs`](../../src/project/files.rs) | `a_plan_is_a_preview_that_touches_nothing` | 测试 |
| INV-FILE-OP-FAILURE-RECOVERS | 文件操作失败时已做的移动按逆序回滚，并如实报告回滚了哪些、没能回滚什么；写文档失败不会留下半个工程 | `C-PROJECT` | [`src/project/files.rs`](../../src/project/files.rs) | `a_failure_after_the_moves_puts_them_back` | 测试 |
| INV-CONFIG-SOURCE-VISIBLE | 构建/运行配置的每个生效值都带来源（内置默认 / 目标 / 本机覆盖）以及它替换掉的各层取值：没有“只看值”的读取路径 | `C-PROJECT` | [`src/project/configuration.rs`](../../src/project/configuration.rs) | `every_value_says_where_it_came_from` | 结构 + 测试 |
| INV-CONFIG-NOT-SHARED | 本机记录的活动目标与覆盖都带工程身份：读取时必须给出身份，属于别的工程的选择与覆盖一律不使用并说明原因 | `C-PROJECT` | [`src/project/configuration.rs`](../../src/project/configuration.rs) | `an_active_configuration_does_not_leak_between_projects` | 结构 + 测试 |
| INV-CREDENTIAL-VIEW-NEVER-MATERIAL | 机密引用视图只呈现“名字 + 材料在哪 + 存储怎么说”，从不读取、持有或渲染材料本身 | `C-PROJECT` | [`src/project/configuration.rs`](../../src/project/configuration.rs) | `a_credential_is_shown_as_a_reference_and_its_health_never_its_material` | 结构 + 测试 |
| INV-IMPORT-PLANS-BEFORE-WRITING | 导入先给候选：每一步迁移与每个会变的成员（diff）都在写盘之前列出，候选本身不写盘 | `C-PROJECT` | [`src/project/import.rs`](../../src/project/import.rs) | `the_candidate_names_the_steps_and_every_member_that_would_change` | 测试 |
| INV-MIGRATION-KEEPS-ORIGINAL | 迁移先留底并逐字节校验留底内容，再写结果并读回；任何一步失败都保证工程保持原样（失败发生在写之后则把原文放回） | `C-PROJECT` | [`src/project/import.rs`](../../src/project/import.rs) | `committing_keeps_the_file_it_read_and_the_result_reads_back` | 测试 |
| INV-VALIDATION-NEVER-WRITES | 工程校验只回答不写盘：缺失的 SDK/资源被逐条列出，文档既不被清空也不被改写，且每次运行都把「文档逐字节未变」当作结论念出来 | `C-PROJECT` | [`src/project/validate.rs`](../../src/project/validate.rs) | `validating_a_project_with_nothing_present_is_a_report_and_not_an_edit` | 结构 + 测试 |
| INV-MISSING-IS-NOT-NOT-CHECKED | 「没有看过」与「不在那里」是两条不同结论：未读工具链、许可被拒、凭据存储不可达都报 not-checked，只有真查过而缺失的才报 missing，越出工程根的引用报 refused 而不是 missing | `C-PROJECT` | [`src/project/validate.rs`](../../src/project/validate.rs) | `not_looking_at_the_toolchain_is_not_the_same_finding_as_a_missing_sdk` | 测试 |
| INV-OFFLINE-OPEN | 打开工程不向任何远端提问：每条需求的答案都带来源，来源由需求类别唯一决定，而没有任何类别的来源是网络 | `C-PROJECT` | [`src/project/validate.rs`](../../src/project/validate.rs) | `every_answer_says_where_it_came_from_and_none_of_them_needs_the_network` | 结构 + 测试 |
| INV-RESCAN-IS-FRESH | 重扫是重新读文件而不是复用记住的状态：记录的扫描只作对照，条目按文档写出的路径标识（一个文件的多个成员只算一条），属于别的工程或来自更新构建的记录一律拒绝 | `C-PROJECT` | [`src/project/validate.rs`](../../src/project/validate.rs) | `a_re_scan_names_what_appeared_what_vanished_and_what_nothing_touched` | 测试 |
| INV-WORKSPACE-KEYED-BY-MEMBER | 工作区里每个文件由「成员 + 该成员写出的路径」标识：同一路径在两个成员中是两个文件；离开本成员仍留在工作区内的引用按它落地的成员命名，越出工作区的引用被拒绝，落在工作区内却没有成员拥有的路径单独报告 | `C-PROJECT` | [`src/project/workspace.rs`](../../src/project/workspace.rs) | `a_file_is_identified_by_its_member_and_not_by_its_path_alone` | 结构 + 测试 |
| INV-MEMBER-REMOVAL-TOUCHES-NOTHING | 添加与移除成员只改工作区文档：不创建也不删除任何目录；被别的成员文档引用时先阻断并逐条列出引用；写之前比对磁盘上仍是读到的那一份，冲突则一个字节都不写 | `C-PROJECT` | [`src/project/workspace.rs`](../../src/project/workspace.rs) | `removing_a_member_writes_the_document_and_touches_no_directory` | 测试 |
| INV-WORKSPACE-ACTIVE-IS-MACHINE-LOCAL | 活动成员是本机状态：按工作区身份记录在宿主配置目录，共享文档不因切换成员而改变；记录里的成员已经不在了则如实报告，不改指另一个成员 | `C-PROJECT` | [`src/project/workspace.rs`](../../src/project/workspace.rs) | `the_active_member_is_machine_state_and_belongs_to_one_workspace` | 结构 + 测试 |
| INV-KIT-IS-NOT-AN-OUTPUT | Kit 只描述工具组合，从不描述输出：文档里没有 profile/triple 作为它自己的属性（写进来会被读入时拒绝），输出只以 triple 提问，成员各自显式声明能服务哪些 triple | `C-PROJECT` | [`src/project/kit.rs`](../../src/project/kit.rs) | `a_kit_is_not_an_output_and_a_profile_in_it_is_refused` | 结构 + 测试 |
| INV-KIT-DIAGNOSES-EXPLICITLY | 缺成员、版本不符、无法服务与不兼容组合都显式给出，且绝不互相冒充：「没人问过」不是缺，无法比较的版本不判满足也不判不满足，必需角色没有成员时明说无法构建 | `C-PROJECT` | [`src/project/kit.rs`](../../src/project/kit.rs) | `not_asked_a_missing_member_and_a_wrong_version_are_three_different_findings` | 结构 + 测试 |
| INV-KIT-IS-VERSIONED-AND-KEYED-BY-ROLE | Kit 自带版本（两个版本是两个 Kit），成员按角色唯一：同一角色两个成员、角色不认识、版本要求读不出来、路径探针配版本要求，都在读入时拒绝，不留到解析时按「找到哪个算哪个」决定 | `C-PROJECT` | [`src/project/kit.rs`](../../src/project/kit.rs) | `a_document_that_cannot_be_a_combination_is_refused_when_it_is_read` | 测试 |

## 不变量的补充说明

| 编号 | 说明 |
| --- | --- |
| INV-SINGLE-WRITER | 视图只借用模型且没有 &mut 方法；快照的字段私有且没有写入方法。写入入口被 writers 声明并逐名校验，新增入口必须同时更新数据文件 |
| INV-ATOMIC-CHANGE | 校验与暂存发生在只含被触及节点的 overlay 上，全部通过才提交 |
| INV-INVERTIBLE | 属性逆操作在另一条用例中覆盖：setting_a_property_inverts_to_restoring_or_clearing_it |
| INV-STALE-BASE | 拒绝结果是值，携带 base 与 current 两个版本 |
| INV-IDENTITY-STABLE | 身份由 id 分配器裁定（identity.rs 的 a_retired_id_is_still_known_but_not_live 覆盖退休语义），是否复用由 store 的 apply 校验：被移除的身份被拒绝（本用例），未申请过的身份同样被拒绝（a_node_added_by_an_invented_id_is_refused） |
| INV-DEFAULT-DENY | 权限表逐行对照白皮书 §12.5 与 src/contracts/permission.rs 的 Capability::default_grant |
| INV-DENIAL-EXPLAINED | 变更集层面的同一条约束由 every_rejection_names_itself_uniquely_and_explains_itself 覆盖 |
| INV-STALE-LABELED | 快照与源映射各自带 stamp，可被同一套 Staleness 判定 |
| INV-DIAGNOSTIC-LOCATABLE | 位置同时携带节点与源码区间，两者不是二选一 |
| INV-MOST-SPECIFIC-MAPPING | 未映射的节点与位置另有用例覆盖 |
| INV-EFFECT-DECIDED | 命令、文件与网络分别由 run-cargo-check、write-file 与 request-network-service 裁定，映射理由写在 EffectClass 的文档注释里；部署仍是未决定（C-REMOTE 为延期能力）并给出原因，宿主因此拒绝，而不是发明一条授权 |
| INV-VERSION-UNKNOWN-NOT-WRITTEN | 未知版本不会成为 ProjectDocument，而只有 ProjectDocument 才能渲染，因此“不写回”是类型结构而不是纪律；读取时只尽力读身份，其余成员原样列出 |
| INV-NO-FIELD-LOSS | 唯一例外是凭证引用与其来源：那里的成员集合是封闭的，出现未知成员按“机密被写进工程”拒绝，而不是保留 |
| INV-NO-HALF-PROJECT | 暂存目录在目标同级（同一文件系统），因此提交是一次 rename：要么工程存在，要么原目录与之前完全一样；取消后再次取消会如实报告“没有暂存副本” |
| INV-NO-OVERWRITE | 空目录可以承接新工程（rename 允许替换空目录），但非空目录、文件与缺失的父目录都各自给出准确拒绝；“向导打开期间别人写进目标目录”这一冲突由提交失败覆盖 |
| INV-CREATED-IS-READABLE | 暂存目录里的 project.json 与每个页面文档都由设计器使用的读者读一遍（verify_staged），因此写出器与读码器不一致会让创建失败，而不是交付一个打不开的工程 |
| INV-OPEN-DECIDES-NOTHING | 授权记录在打开后为空；未信任时 may_write 为假；工具链状态是 NotAsked。路径不是权限：只有 ProjectSession::decide 会记录信任 |
| INV-RECENT-IS-A-PLACE | recent 模块只持有路径/名字/身份/时间，没有权限服务也没有启动程序的能力；修整（repair）按身份判定，不同工程拒绝，路径仍在则不修 |
| INV-PROMPT-IS-CURRENT | 提问的 findings 由目录本身读出（build.rs / .cargo/config / rust-toolchain*），因此“提问后新增 build.rs”会被发现；拒绝与撤销两次都记录在授权记录里 |
| INV-MIGRATION-IS-VERSIONED | 决策 Q6：迁移前不原地覆盖原文件（save 对旧版本拒绝，save_migrating 先写 .v1.bak）；决策 U1：只向前、按步；文档声明 v1 却带 v2 才有的成员会被拒绝，而不是被相信 |
| INV-FILE-OP-PREVIEWS | 阻断项（blockers）与计划同一次读出，因此“能不能做”先于“做了什么”；删除一个页面文档被阻断，指向它的设计引用逐条列出（页面成员与属性），并给出替代做法：删条目而不是删文件 |
| INV-FILE-OP-FAILURE-RECOVERS | 移动先做、文档最后写，因此失败只可能落在文档写入上；FileFailure::is_recovered 说“是否还需要人看一眼”，命令的退出码不因此变成成功（原则 22：不把失败藏在成功里） |
| INV-CONFIG-SOURCE-VISIBLE | Setting 同时持有 value、source 与 replaced，因此调用方拿到值就拿到来源；某一层沉默不等于它说了“无”——沉默时下层决定，报告据实写出（quiet 目标用例）；本机覆盖替换列表时也逐条报告被替换项，避免“被去掉的特性”看起来像“从来没要过” |
| INV-CONFIG-NOT-SHARED | active_for(id) 只回答该身份；别处的选择返回 AnotherProject 并把双方身份都写出来，解析时不采用；覆盖按 工程+目标 判定，未命中的进 Skipped 并带原因；ResolvedConfiguration::for_project 拒绝把配置交给别的工程 |
| INV-CREDENTIAL-VIEW-NEVER-MATERIAL | 凭据视图由 CredentialRef::health(store, now) 得出（该接口只回答 Usable/ExpiringSoon/Expired/Missing/Unavailable）；lookup 只在网络层发出请求前调用；本模块的类型里没有 Secret 字段，因此“渲染出机密”不是纪律而是不可表达 |
| INV-IMPORT-PLANS-BEFORE-WRITING | 候选由 migration_path 与「读到的 JSON」对「本构建会写出的 JSON」的差异算出，两者都只读；diff 不列出没有变化的成员（否则没人读），容器只报形状与规模；候选不写盘这条由用例直接断言（文件原文未变、备份不存在） |
| INV-MIGRATION-KEEPS-ORIGINAL | 顺序是有意的：先比对磁盘上的文件仍是读到的那份 → 已有留底则拒绝（绝不覆盖留底）→ 写留底并读回逐字节比对 → 迁移 → 读回要求是当前版本；写失败或读回失败时用留底把原字节放回，放回也失败才报告 left_behind；已有留底与外部改动两个用例断言「工程一动没动」 |
| INV-VALIDATION-NEVER-WRITES | 结构：Validation::of 只取已读入的文档、已诊断的工具链状态与已查过的凭据视图，本模块里没有任何写入者；测试：同一工程缺资源又缺 SDK 时先断言文件字节未变，再走既有编辑路径（with_resource + write_document）断言页面/目标/服务/身份都还在；命令行每次收尾都重读项目文件逐字节比对，是本条已有的第二处证据 |
| INV-MISSING-IS-NOT-NOT-CHECKED | Availability 的四个取值合并任意两个都会把「没查」说成「没有」，所以是四个而不是两个；工具链映射取 ToolchainState 自己的话：NotChecked 一律 not-checked（没人问/工程未信任/许可拒绝），Checked 里没答上的程序才是 missing；凭据同理（存储不可达是 not-checked，环境变量没设是 missing）；路径层拒绝的引用是 refused |
| INV-OFFLINE-OPEN | Where::of(kind) 是全函数：文件→工程自身文件，工具链与目标→本机，凭据→本机凭据存储，网络这一取值在 ALL 里但没有类别映射到它，所以「离线打开同一条路径」是类型的性质而不是散文；报告把来源折成一行念出来，并在真有网络需求时改口（不是一句永远成立的 yes）；用例断言 needs_no_network() 与「每个需求的 source 都不是 Network」，并在无网络命名空间（unshare -rn，curl 取不到主机）里跑过一次真实调用链 |
| INV-RESCAN-IS-FRESH | Scan::read 重新打开并扫描（用例断言它与「先前读入的文档」在文件出现后给出不同答案）；Scan::of 取文件声明的版本（migrated_from 优先）而不是文档值自带的版本——文档值自带的版本是本构建解释出的版本，这条陷阱以前栽过一次；Scanned::key 是文档写出的路径，因为状态是路径的性质而不是成员的，所以同一文件被多个成员命名时只有一条；Scan::between 先比对工程身份，身份不同的记录被拒绝而不是当成对照 |
| INV-WORKSPACE-KEYED-BY-MEMBER | WorkspaceFile 的地址就是 member:path，解析时没有成员一律拒绝（两条同名的文件在别的层看来完全一样）；文件表按（成员, 路径）去重，所以页面文档与 startup 指向同一文件时是一行；CrossRoot 四态：WithinMember / AnotherMember{成员,落地路径} / Outside{原因} / NoMemberOwns{路径}，最后一条正是「加成员」能回答的；判断是否留在本成员用路径层自己的 resolve_for_creation（同一条越界规则，且不要求父目录已存在），因此「即将创建的文件」不会被读成「跑到别处去了」 |
| INV-MEMBER-REMOVAL-TOUCHES-NOTHING | 成员是文档里的一行，所以移除不碰目录，用例在移除后逐字节比对那个目录里原来的文件；添加只接受一个「已经在那里的目录」（命名一个 root 不等于创建它，造目录属于创建路径）；被引用时的阻断项与引用一起给出（Referrer），不做静默修正；受影响时不写盘由用例直接断言（文档原文未变）；写路径是唯一一次写：先比对 original 再原子替换，失败即「工作区原样」而不是半个状态 |
| INV-WORKSPACE-ACTIVE-IS-MACHINE-LOCAL | ActiveMember 四态（Named / MemberIsGone / AnotherWorkspace / NeverChosen）与 T02.06 的活动目标同一套思路与同一套判定词；WorkspaceState 是按工作区身份键控的本机文件（未写入者不写盘、原子写、更新构建的记录拒绝读），用例断言激活后工作区文档里没有任何「活动成员」字样；未知成员名被拒绝而不是记录一个空选择 |
| INV-KIT-IS-NOT-AN-OUTPUT | Kit::diagnose 只接一个 triple，没有任何参数是 profile；成员用 Serves（Every 或一组 triple，两种都写出来，不靠省略字段表达「都行」）；不认识的成员在读入时被拒绝并说明「哪个 profile 由工程配置回答」，因此「把 profile 塞进 kit」不是被忽略而是报错；报告的每一行都写着这一点 |
| INV-KIT-DIAGNOSES-EXPLICITLY | Finding 五态（Ok/Missing/Version/NotChecked/CannotServe）合并任意两个都会把「没查」说成「没有」或把「版本不符」说成「没装」；Observed 里没有记录的成员返回 NotChecked 而不是 Nothing，理由原文保留；Requirement::holds 对无法比较的版本返回 None → NotChecked，并把版本与要求都写出来；必需角色（只有 toolchain）缺成员时报告里带「nothing can be built without one」；命令的退出码看 has_decided_problems（被判定为错），不看 is_complete，所以「没查过」不会冒充失败 |
| INV-KIT-IS-VERSIONED-AND-KEYED-BY-ROLE | identity 的三个成员是 id/name/version，version 为空即拒绝；角色用 ProjectError::Duplicate 拒绝第二个同角色成员（两个 toolchain 是提问而不是组合）；版本要求只有 `*`、`1.2.3`、`>=1.2.3` 三种形状，其它形状在读入时报出写法；路径探针的「版本」只能是 `*`，因为路径无法回答版本，能检查的要求才准写进去 |

## 为什么权限表要交叉校验

权限表是唯一一处"代码照着文档抄"的地方，也是最不能漂移的地方：它决定 agent 能不能改文件、能不能
执行构建。白皮书 §12.5 是决策出处，`Capability::default_grant` 是可执行版本，两者由检查脚本逐行
比对（操作类别、默认权限两列必须逐字一致，行的顺序也必须一致）。`src/contracts/permission.rs` 里
另有一条测试从代码一侧读 `docs/plan/contracts.json`，把"操作 → 授权 → 拒绝原因"三元组逐条核对，
所以三个方向都闭环：白皮书 ↔ 数据文件 ↔ 代码。

## 证据强度怎么读

| 证据写法 | 含义 |
|---|---|
| 结构 + 测试 | 类型层面不允许（例如视图没有 `&mut` 方法），并且有测试覆盖可运行的那部分 |
| 测试 | 行为由用例断言；用例只证明它写过的路径，不证明不存在别的路径 |
| 测试 + 交叉校验 | 行为由用例断言，且该用例依赖的表与白皮书/数据文件逐行一致 |

「唯一可写模型」是**结构 + 测试**：视图只借用模型且不暴露任何写入方法，快照字段私有且没有写入
方法；同时所有写入入口被 `writers` 逐条声明、由检查脚本对照源码枚举。检查脚本能发现的错误是
"新增写入入口却没有声明"；它发现不了的错误是"声明表本身写错了分类"——后者由人复核。
