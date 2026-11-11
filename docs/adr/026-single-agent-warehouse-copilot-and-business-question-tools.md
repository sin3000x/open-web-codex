# ADR-026：仓网只保留单 Agent Copilot，规划 Tool 以业务问题为粒度

状态：Accepted
日期：2026-11-11

局部替代 [ADR-019](019-task-selected-copilot-packages-and-shared-tools.md) 第 3 节“单 Agent 和多
Agent 是两个独立 Copilot”，以及 [ADR-024](024-warehouse-copilot-contract-simplification.md) 中
由模型逐步串联路线、成本、基线与求解 Tool 的运行合同。Task 显式选择 Copilot 包、根级共享 Tool
注册表、Workspace/Runtime/MCP/Artifact 的所有权，以及 ADR-023/025 的准备输入合同不变。

## 背景

仓网同时存在单 Agent 与多 Agent 两个 Copilot 包。多 Agent 包是 Supervisor → Data → Network 的
直线交接：没有并行，也没有依靠上下文隔离解决任何问题，只增加交接跳数和“逐字转交 child 结果”
的规则。它没有使用者，其运行门禁在能力基线中已记录为未通过，两套提示词也已出现漂移。

单 Agent 包的 Root Skill 实质上是一段写在提示词里的固定流程。回答“现状/基线”需要模型依次调用
路线矩阵、成本矩阵和基线评估三个 Tool，参数全部固定，模型唯一的工作是把上一步的引用原样传给
下一步。每一步都是一次模型往返；Skill 中大量“只调用一次”“原样复制”“不要再试”之类的禁令，都在
补偿模型照流程执行时出现过的偏差。流程一旦调整，步骤表、参数说明和 Tool 搜索表必须同时修改。

## 决定

### 1. 仓网只保留单 Agent Copilot

删除 `copilots/warehouse-network`（包 ID `warehouse-network-copilot`）及只为它存在的门禁与剧本。
`copilots/warehouse-network-single-agent` 是唯一的仓网 Copilot。现阶段不为多 Agent 协作维护
端到端门禁；Platform 中已有的原生协作投影保持原样，但不计为已验证能力，其状态由能力基线记录。

验证 Platform 如何为 child Role 生成 MCP 与 Skill 配置的单元测试使用测试自带的样例包，不读取任何
产品包的内容。

### 2. 规划 Tool 以业务问题为粒度

Planner 向模型公开三个分析 Tool，每个回答一类业务问题，并在一次调用内完成所需的路线、成本、
基线与对比：

| Tool | 业务问题 | 仓库范围 |
| --- | --- | --- |
| `evaluate_baseline` | 现有网络的时效达标率与成本 | 现有仓 |
| `evaluate_scenario` | 新增和/或关闭一组指定仓库后的变化 | 现有仓加被新增的候选仓 |
| `find_sites_for_service_target` | 达到服务目标所需的最少新增仓及其组合 | 全部仓 |

三者共用同一组输入：服务时限、可选的路线口径与成本口径、可选的准备输入路径。

- 路线矩阵、成本矩阵与方案对比是 Tool 内部步骤；它们的 Resource 引用不再经过模型。原有的
  `prepare_route_matrix`、`plan_cost_matrix`、`evaluate_network_baseline`、
  `assess_facility_change`、`solve_p_median`、`compare_network_scenarios` 与
  `import_navigation_matrix` 不再注册为 MCP Tool，只作为 Planner 内部函数保留。
- 未给出准备输入路径时，Tool 按该业务问题固定需要的数据角色，在当前 Workspace 的准备输入中解析
  唯一可复用的一份，并在结果中写明所用路径；没有可复用输入或存在多份不同输入时返回 typed 结果，
  不猜测、不自动准备。
- 路线或报价存在缺口且调用方未给出口径时，Tool 返回一次 `needs_input`，同一张输入卡片包含全部
  待选问题；用户选择后以同一 Tool 加上所选口径重新调用。
- `evaluate_scenario` 接受仓库 ID 或与准备输入中完全一致的仓库名称；名称对应多个仓库或没有对应
  仓库时返回 typed 错误，不做模糊匹配。本阶段不公开“搬迁仓库”。
- 结果直接包含基线、方案、对比与按仓库拆分的明细，并只对可供后续地图与报告使用的结果发布
  Resource 引用。

真实导航仍是独立的少数分支：`create_navigation_matrix_request` 与地图服务的执行 Tool 保持不变，
导入步骤并入三个分析 Tool 的路线口径。地图数据准备、报告发布与全部 Data Tool 不变。

## 被否决的替代方案

- **保留多 Agent 包作为冻结样例。** 它依赖将被移除的底层 Tool，冻结意味着要么继续同步维护，要么
  任其失效，两者都违背“不并行维护被取代路径”。
- **同时公开业务 Tool 与底层 Tool。** 模型会在两条路径之间选择，Skill 必须同时解释两套合同，
  原有的偏差仍会出现。
- **只改提示词、继续由模型串联底层 Tool。** 这是此前反复采用的做法，每次新增一条禁令，没有消除
  往返次数和传参偏差的根因。
- **把地图卡片也并入分析 Tool。** 地图属于另一个 Tool 包并有独立的交付合同，合并会跨越包边界。

## 后果与验证

- 一次基线问题的 Tool 调用从“发现、解析、路线、成本、评估”五次降为一次；模型只保留问题分类、
  数据准备时的字段判断和面向业务的解释。
- 超出三类分析与数据问答的问题，由 Root Skill 规定如实说明当前能力范围；模型不再拥有可自由拼接
  的底层计算 Tool。
- 现阶段没有任何多 Agent 仓网能力。重新引入时必须按届时的 Runtime 合同重新设计，不得恢复本次
  删除的包。
- Planner 单元测试覆盖三个分析 Tool 的 ready、`needs_input`、准备输入解析与 typed 拒绝路径，并
  保留内部函数的既有测试；注册清单测试固定公开 Tool 集合。确定性端到端门禁与真实 Provider
  门禁使用单 Agent 剧本。当前实现与验证状态以[系统架构](../architecture.md)和
  [能力基线](../capability-baseline.md)为准。
