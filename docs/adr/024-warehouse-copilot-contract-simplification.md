# ADR-024：仓网双 Copilot 的最小运行合同与精确求解边界

状态：Accepted

日期：2026-08-21

补充 [ADR-019](019-task-selected-copilot-packages-and-shared-tools.md) 的共享 Tool 构建与安装状态，
并局部替代 [ADR-023](023-workspace-prepared-inputs-and-domain-geospatial-execution.md) 中 Data 检查
Resource 和跨输入 pair 复用的实现形态。ADR-018/019/023 的 Workspace、Runtime、MCP、Artifact 与
权限所有权继续有效。

## 背景

单 Agent 与多 Agent 仓网 Copilot 已共享同一组 Data、Planner 和 Maps Tool，但运行闭环仍有五类
重复或不精确的机制：每个 package fingerprint 重建同一 Tool 环境；Platform 用 Role/MCP 探测制造
无法保证执行成功的 `Ready`；同一业务方法被拆成过多 Skill；Data 检查先发布临时 Resource；路线
scope 只能在“现有仓”和“全部仓”之间选择。最小可行仓数又通过逐个候选仓数重复调用 CP-SAT，
增加轮次、时间和中间状态。

这些机制既不能扩大 Codex Runtime 的真实能力，也让 Agent 容易把预览、探测或宽泛矩阵误认为完整
业务事实。因此需要在各自 owner 内收敛合同，而不是继续补提示词或 Platform workflow。

## 决定

### 1. SDK 共享构建，package descriptor 仍是运行引用 owner

Copilot SDK 在显式 `build_store_root` 下按 Tool preparation fingerprint 原子构建一次。单 Agent 与多
Agent package descriptor 只保存对已发布 build 的引用；pip/npm 下载缓存跨 fingerprint 稳定共享。
GC 只删除没有被任何 prepared descriptor 引用的旧 build。构建失败不得启动旧 descriptor，也不得
把失败伪装成可用；原 descriptor 仅作为可审计的上次成功记录保留。

### 2. Platform 只陈述安装与配置，不推断执行 readiness

Copilot 状态只有 `Installed`、`Configured`、`Unavailable`、`Failed`。一次状态请求最多执行一次
Runtime `skills/list` 并复用结果；Platform 不扫描历史 child Thread 的 MCP，不保存 Role/MCP 可用性
投影，也不声明 `Ready`。Task 准入只由 active、source revision 和 configured revision 决定；真正的
Role、Skill、MCP 与 Tool 可用性由 Codex Runtime 在当前 Task 执行时给出 typed 成功或失败。

### 3. Skill 只拥有业务方法，强制规则留在 typed Tool

两个 Copilot 共保留八个 Skill：多 Agent 的 Supervisor、Data、Network Planning、Map Delivery，
单 Agent的 Root、Data Preparation、Single Network Planning、Map Delivery。route、analysis 与
optimization 合并为一个 Network Planning 方法。Workspace 输出目录、create-new、源身份、禁止覆盖
和输入 schema 由 Tool 校验，不在多个 Skill 重复。Role TOML 的 MCP 权限和 manifest delivery 仍保持
显式，不增加继承层。

### 4. Data 检查是有界握手，准备文件是持久化业务输入

`inspect_workspace_sources` 直接返回有界 source profile、`preview_sample_count`、`total_count`、
`total_count_exact`、所选相对路径和 `inspection_identity`，不发布临时 Data Resource。
`inspection_identity` 绑定排序后的相对路径和每个完整文件的内容摘要；`prepare_network_input` 在读取
完整数据前重新计算并严格比较，变化时返回 `source_inspection_changed`。检查结果只用于本次握手，
不成为 Platform 数据对象；持久化 owner 仍是 create-new 的 `prepared_network_input.v1` Workspace
文件。

### 5. Network comparison 和路线范围使用一个当前 typed contract

`network_plan_comparison.v2` 接受同一 prepared identity 下任意 assignment-bearing baseline、scenario
或 facility-location before/after 组合。地图和报告只消费公共 assignment、active warehouse 与相对
warehouse changes；非法身份、缺失 assignment 或不一致计算在 schema/Tool 边界拒绝。

`warehouse_scope` 是 discriminated union：`existing_only`、带精确 `candidate_ids` 的
`existing_plus_candidates`、`all_warehouses`。路线、导航和成本矩阵都持久化规范化后的精确
`warehouse_ids`。同一仓集合只有一种范围表示：列出全部候选仓的 `existing_plus_candidates`
在工具边界规范化为 `all_warehouses`，因此下游范围比较等价于仓集合比较。baseline 使用现有仓；
单一设施变化使用现有仓加明确候选；只有全量选址使用全部仓，范围不符时 `solve_p_median` 返回
`p_median_route_matrix_requires_all_warehouses_scope` 或 `p_median_cost_matrix_requires_all_warehouses_scope`。
prior pair facts 只有在 prepared identity、方法、方法参数和精确 warehouse set 全部一致时才可复用。
这条规则替代 ADR-023 中“输入身份不作为复用门”的旧决定；当前实现不保留双读或跨身份兼容。

### 6. 最小仓数最多使用两个 solver stage

`minimum_feasible` 第一阶段在 coverage 约束和上限内最小化新增候选仓数；第二阶段固定第一阶段选定的
仓数并最小化成本。两个阶段共享一个总时间预算，结果只保存 `selected_number_to_open`、
`minimum_number_to_open_proven` 和最多两个 `solver_stages`。第一阶段仅得到 feasible 解时可以继续提供
该解，但必须标记 `feasible_only`，不得声称最小仓数已证明。`exact` policy 仍只执行固定仓数的成本
优化。

### 7. 确定性门验证机制，真实门验证自然业务结果

Provider/Auth/Workspace/cleanup/timeline 由共享 E2E harness 拥有。严格拓扑、Tool 次数与顺序属于
确定性 Platform gate；真实 DeepSeek gate 使用自然业务请求，只验收 typed 业务结果、权限、输出目录、
未知 Tool、断流和最终交付。模型随机性可在确定性门全绿时有界重试；当前 Turn Tool 不可见、错误权限、
Resource 身份错误、服务崩溃或已接受 Tool 后断流属于机制失败，必须修 owner，不能用重试掩盖。

## 持久化与退出条件

- SDK build、缓存和 descriptor 位于 Platform 管理的本地构建根；缓存可删除，descriptor 引用的 build
  不可被 GC。
- Data inspection 结果和 readiness 探测不持久化；prepared 输入、Network Resource 与最终 Workspace
  文件分别由 Workspace、MCP provider 和 Artifact/Workspace 合同持久化。
- 旧 12-Skill catalog、`Ready` 字段、Data inspection Resource、comparison v1、字符串 scope、逐仓数
  `search_attempts` 在全部调用方原子升级后立即删除；没有兼容期或运行时迁移分支。
- 若未来官方 Codex Runtime 或公共 Copilot SDK 提供等价的 typed package build/discovery 合同，应在
  保留 Task/package owner 和失败语义的前提下删除项目 SDK 的对应实现，不在两层并行维护。

## 被否决的方案

- 继续为每个 Copilot package 构建相同依赖：浪费磁盘和启动时间，也制造重复状态。
- 用历史 child Thread、MCP 列表或 Provider 名称推断 readiness：无法证明当前 Turn 可执行。
- 通过 Skill 反复强调路径、source identity 或 Tool 顺序：把 typed 缺口变成提示词概率问题。
- 为 Data preview 发布临时 Resource：增加一次调用和一个没有持久价值的身份层。
- 单一设施变化默认构建全部候选仓矩阵：扩大费用与失败面，并允许无关 pair 被错误复用。
- 逐个仓数调用 solver 或由 Agent 外循环：增加 Tool 轮次，无法共享一个明确的总时间预算。

## 验证

- SDK：共享 fingerprint、并发原子发布、失败保留、descriptor 引用保护和稳定缓存；
- Platform：一次 Skill discovery、无历史 child MCP 探测、Task revision 准入；
- Data：preview/total 区分、完整文件变化拒绝、preview 外候选仍进入准备输入；
- Planner/Maps：comparison 全组合、精确 candidate scope、严格 pair reuse、导航 round-trip；
- Solver：两阶段最优性、stage timeout、unavailable 与 exact policy；
- Copilot：八个 Skill、单 Agent 无 child、多 Agent 只有 Root 直连 Data/Network child；
- E2E：单 Agent 90% 需求加权覆盖和多 Agent Balikpapan 时效变化率各至少一次真实 DeepSeek 通过。
