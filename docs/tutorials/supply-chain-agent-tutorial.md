# 第三篇：从业务问题到可复核的仓网建议

## 场景与决策

华东区有上海、南京两个现有仓。业务希望把一日达覆盖率提高到 90%，并提供杭州、
无锡两个候选点。运营负责人需要在评审会上回答：

1. 当前履约和当前仓网分别是什么水平；
2. 不开新仓、只调整分配是否足够；
3. 增加杭州仓会带来多少覆盖提升和建模成本变化；
4. 在给定候选点中，达到 90% 最少新增几个仓；
5. 这些数字是否足以直接做投资决策。

本篇的目标不是展示一长串已实现能力，而是交付一份可以复核的建议。完成后，预期
结论是：

```text
在演示数据和给定候选点内，杭州是达到 90% 目标的最少新增仓方案。
现有仓严格满足容量时覆盖 70%；增加杭州仓后覆盖 95%，提升 25 个百分点。
建模总成本从 895.10 CNY 增至 1067.10 CNY，增加 172.00 CNY。
这只是给定候选、路线、容量和有限成本项下的规划结论，不能直接替代投资决策。
```

开始前请完成[第二篇](hello-agent-team.md)。返回
[教程总入口](../multi-agent-development-tutorial.md)。

## 第一步：先把业务口径写清楚

不要从“创建几个 Agent”开始。先固定这次决策的口径：

| 项目 | 本教程采用的定义 |
| --- | --- |
| 规划对象 | 上海、苏州、杭州共 100 个需求单位 |
| 现有仓 | 上海、南京 |
| 候选仓 | 杭州、无锡 |
| 服务目标 | 一日达覆盖率至少 90% |
| 一日达 | 端到端时效不超过 86,400 秒 |
| 优化优先级 | 先最少新增仓，再比较建模总成本 |
| 币种 | CNY |

这里的“一日达”不是地图导航时间小于 24 小时。演示合同使用：

```text
端到端时效
  = 截单等待
  + 仓内处理
  + 导航运输
  + 末端缓冲
```

如果这个公式、需求周期或成本范围没有先固定，不同方案的覆盖率与成本就无法比较。

## 第二步：准备环境

如果上一章还没有准备环境，运行：

```bash
tools/supply-chain-network-planner/bin/setup-env
```

如果第二篇已经成功准备环境，不必重复运行。新建真实任务后，展开侧栏
**MCP Servers**，确认 `supply_chain_data` 与 `supply_chain_planner` 都是 `ready`。
如果不是，先回到第二篇末尾的排错检查。

## 第三步：准备可追踪的数据基线

在真实 Codex 模式下新建任务，发送：

```text
我们要评估华东一日达仓网。

请使用 $prepare-planning-dataset 检查并读取
source_id `warehouse-network-fixture`，构建并验证 planning-dataset.v1。

先只回答：
- 数据覆盖的规划周期、订单行数和需求总量；
- 上海、苏州、杭州的需求分布；
- 有实际时效记录的需求量和历史一日达比例；
- 促销与缺失数据警告。

不要开始推荐候选仓。
```

Agent 应先 `inspect`，再 `build`，最后 `validate`。预期数据事实：

| 事实 | 值 |
| --- | ---: |
| 订单行数 | 6 |
| 总需求 | 100 |
| 上海 / 苏州 / 杭州 | 40 / 35 / 25 |
| 有实际时效记录 | 90 |
| 历史一日达需求 | 55 |
| 历史一日达比例 | 55 / 90 ≈ 61.1% |
| 促销相关需求 | 30 |
| 缺少实际时效 | 10 |

### 为什么数据职责不直接给选址结论

数据准备知道哪些订单存在、怎样聚合、哪些字段缺失；它不拥有候选点、路线和选址
目标。如果它在同一步里直接说“应该开杭州仓”，结论就无法区分：

- 来源数据事实；
- 规划模型假设；
- 候选方案计算；
- Agent 的解释。

Data Tool 因此交付一个经过验证的 `planning-dataset.v1` Resource。后续步骤必须引用
同一版数据，而不是在消息中重新概括后再凭概括计算。

## 第四步：建立本次规划的快照与路线

仓网计算还需要候选仓与路线。教程提供两个可人工检查的文件：

- [`network-input.json`](../../tools/supply-chain-network-planner/examples/network-input.json)
  包含两个现有仓、杭州和无锡候选仓、容量、费率与服务政策；
- [`route-matrix-input.json`](../../tools/supply-chain-network-planner/examples/route-matrix-input.json)
  包含 4 个设施到 3 个需求点的 12 条演示路线。

在生产场景中，候选点是规划人员的显式输入，路线通常由 `map_utils` 的导航能力获得。
本教程使用固定路线，避免地图密钥和实时路况影响第一次学习。

继续发送：

```text
请准备本次仓网规划基线。

以刚才 planning-dataset.v1 中的 network_input 为需求、现有仓、规划周期、
币种和服务政策的来源。再从
`tools/supply-chain-network-planner/examples/network-input.json`
加入杭州和无锡候选仓及其费率；不要修改已有需求和现有仓事实。

使用
`tools/supply-chain-network-planner/examples/route-matrix-input.json`
中的 12 条演示路线。

创建 network_snapshot.v1 和 route_matrix.v1，并分别验证。
如果共享字段与 Dataset 不一致，停止而不是自动选择一个版本。
```

这里的 **Snapshot（快照）** 是一次计算使用的完整、不可变输入状态。数据、候选、
服务政策或路线有任何变化，都创建新快照，而不是改写旧结果的输入。

**Route Matrix（路线矩阵）** 是每个设施到每个需求点的距离与时长表。本例有 4 个
设施、3 个需求点，所以完整矩阵必须有 `4 × 3 = 12` 条路线。

本篇反复要求“验证后再继续”，可以把它理解为一道验证门：输入或结果没有通过检查，
Agent 就停止，而不是带着错误继续生成建议。

Tool 调用中应出现：

```text
supply_chain_planner.prepare_network_snapshot
  → supply_chain_planner.register_route_matrix
  → supply_chain_planner.validate_network_resource
```

路线必须覆盖每个设施—需求点组合。缺一条路线时，Agent 应报告缺口；它不能默默用
直线距离或模型常识补一个时长。

## 第五步：先计算“不开新仓”的基准

发送：

```text
使用已经验证的 Snapshot 和 Route Matrix 计算当前覆盖率。

分别给出：
1. 保持记录中的当前仓库关系时的覆盖率；
2. 不新增仓、严格满足现有仓容量并允许重新分配时的覆盖率。

验证结果，并解释两个数字为什么可能不同。
```

预期：

| 指标 | 覆盖需求 | 覆盖率 | 建模总成本 |
| --- | ---: | ---: | ---: |
| 当前记录关系 | 75 / 100 | 75% | 1038.00 CNY |
| 现有仓优化基准 | 70 / 100 | 70% | 895.10 CNY |

当前记录把全部需求都关联到上海仓，但上海容量只有 40。第一行保留关系并报告超容量；
第二行严格遵守上海 40、南京 30 的容量，所以最多分配 70。后续评价杭州仓时，应与
第二行的“现有仓优化基准”比较，不能把不同口径的 75% 当基准。

## 第六步：评价杭州方案

发送：

```text
请使用 $evaluate-network-scenario 比较两个同口径方案：

- baseline：只启用 warehouse-shanghai 和 warehouse-nanjing；
- add-hangzhou：在相同 Snapshot、Route Matrix 和服务政策下，
  再启用 candidate-hangzhou。

验证两个方案和 comparison，报告覆盖率、覆盖需求、建模总成本及差值。
不要把当前记录关系的 75% 当作 baseline。
```

预期比较：

| 方案 | 覆盖需求 | 覆盖率 | 建模总成本 |
| --- | ---: | ---: | ---: |
| 现有仓优化基准 | 70 / 100 | 70% | 895.10 CNY |
| 增加杭州仓 | 95 / 100 | 95% | 1067.10 CNY |
| 差值 | +25 | +25 个百分点 | +172.00 CNY |

“建模总成本”只包含当前合同中的设施固定成本、处理成本和距离运输成本。它没有包含
库存、建设、税费、关仓、缺货损失、碳成本等，因此不能写成“公司总成本只增加
172 元”。

## 第七步：让目标反推候选仓数量

现在不再指定杭州，改为提出业务目标：

```text
请使用 $optimize-network-to-target，在当前 Snapshot 的杭州、无锡候选点中，
求一日达覆盖率至少 90% 时最少需要新增几个仓。

验证 solution 和 result。报告选择的候选仓、覆盖率、建模总成本、
评估的候选组合数量，以及“最优”结论的适用范围。
```

预期：

```text
状态：达到目标
新增仓数量：1
选择：candidate-hangzhou
覆盖率：95%
建模总成本：1067.10 CNY
评估的候选组合：3
```

求解器先最小化新增仓数量，同样数量时再选择建模总成本更低的方案。这里的“最优”
只表示：

> 在输入的杭州、无锡候选点，给定路线、容量、费率和服务政策下的精确结果。

它没有搜索地图上的任意位置，也没有证明杭州是现实世界的全球最优仓址。

## 第八步：把结果写成决策摘要

最后发送：

```text
请把已经验证的结果整理成一页决策摘要，包含：

1. 建议与适用范围；
2. 历史履约、当前关系、现有仓优化三个不同口径；
3. 杭州方案相对同口径 baseline 的覆盖和成本差值；
4. 90% 目标求解结果；
5. 数据质量警告；
6. 已计入与未计入的成本；
7. 做投资决策前还要验证的三项事实。

每个数字都注明分母、币种或规划周期。不要产生未经 Tool 验证的新数字。
```

一份合格答案应明确区分：

- 历史实际表现：55 / 90，约 61.1%；
- 当前记录关系的模型覆盖：75 / 100，且存在超容量；
- 现有仓严格容量优化：70 / 100；
- 杭州候选方案：95 / 100；
- 投资建议：仍需真实租建成本、库存策略和需求预测等补充验证。

## 为什么这里确实适合多 Agent

做到这一步后，再回看职责会更直观：

```text
Supervisor：对“是否开杭州仓”负责
├── Data Agent：对需求基线、历史履约和数据质量负责
└── Network Planning Agent：对路线、容量、方案计算和求解范围负责
```

**Agent** 是在一个 Thread 中为目标采取行动的执行者；**Skill** 是 Agent 掌握的一套
工作方法；**MCP Server** 提供 Tool 和 Resource；它们不是同一层概念。

一个 Network Planning Agent 可以使用准备基线、评价方案、目标求解、结果验证等多个
Skill。五个 Skill 不代表五个 Agent。

真实协作时，Supervisor 应把经过验证的 Dataset 引用交给规划子 Thread，而不是把
订单摘要复制成一段新的自然语言。它还应拒绝：

- 未通过验证的数据；
- 路线不完整的快照；
- 不同周期或服务政策的方案比较；
- 把有限候选最优夸大成全球最优；
- 把建模成本写成完整财务成本。

## 可选概念：Resource 什么时候要升级为 Artifact

本教程在一个 Thread 中使用 MCP Resource 保存较大的中间结果。Resource 适合当前
MCP 内部重复读取。

当 Data Agent 与 Network Planning Agent 运行在不同子 Thread，而且结果需要长期
身份、授权、版本和保留策略时，应使用平台 **Artifact**：

> Artifact 是独立于某次消息或运行、可以被授权读取和长期管理的成果。

```text
Data Agent
  → planning-dataset.v1 Artifact
  → Network Planning Agent
  → network-simulation.v1 Artifact
  → Supervisor
```

不要把短结论都变成 Artifact，也不要把大型 Dataset 整份复制进 Agent 消息。

## 再做一次业务规则修改

现在做一个贴近业务的小改动。假设投资评审规定：

> 缺少实际履约时效的需求超过总需求 5% 时，数据集不能用于选址决策。

当前演示数据缺失 `10 / 100 = 10%`，所以改造后 Agent 应停止，而不是继续给出杭州
建议。按照下面的顺序修改。

先在
[`test_data_core.py`](../../tools/supply-chain-network-planner/tests/test_data_core.py)
的 `test_builds_planning_dataset_and_network_handoff` 中，把：

```python
assert dataset.data_quality.valid is True
```

改为：

```python
assert dataset.data_quality.valid is False
assert any(
    "10/100 demand units" in item
    for item in dataset.data_quality.errors
)
```

只运行这一个测试文件：

```bash
.local/open-web-codex/tool-envs/supply-chain-network-planner/bin/python \
  -m pytest tools/supply-chain-network-planner/tests/test_data_core.py -q
```

此时应该失败，因为代码仍然只产生 warning。这个失败先证明测试确实覆盖了新需求。

再打开
[`data_core.py`](../../tools/supply-chain-network-planner/supply_chain_planner/data_core.py)，
在 `_analyze` 中把现有的 `if unobserved:` 块替换为：

```python
if unobserved:
    missing_detail = (
        f"{unobserved}/{total_units} demand units "
        f"({unobserved / total_units:.2%}) have no observed delivery duration"
    )
    if unobserved / total_units > 0.05:
        errors.append(missing_detail)
    else:
        warnings.append(missing_detail)
```

再次运行这个测试文件，确认通过；然后运行全部测试，确认其他计算没有被破坏。

检查
[`prepare-planning-dataset/SKILL.md`](../../tools/supply-chain-network-planner/skills/prepare-planning-dataset/SKILL.md)
会发现它已经要求 Agent 不得把带验证错误的数据交付为可决策结果，所以这里不需要
修改 Skill。最后新建任务，再请求评估 `warehouse-network-fixture`。正确结果是报告
`10%` 缺失超过门槛并停止选址，而不是继续输出杭州方案。

这个练习同时保护三层职责：

```text
普通代码计算缺失比例
  → Tool 返回 blocking error
  → Skill 规定停止交付
  → Agent 向用户说明缺什么
```

这里不需要修改 Tool 签名、Skill、Agent 职责，也不需要新增 MCP Server：输入输出
结构和职责都没有变化，只是 Data Tool 内部的质量规则变了。这正是先判断“变化属于
哪一层”的价值。

## 常见失败与判断

| 现象 | 先回到哪一步 |
| --- | --- |
| 总需求不是 100 | 数据来源、聚合和 Dataset 对账 |
| 历史履约率分母用了 100 | 检查 10 个缺失实际时效的需求 |
| 当前 75% 与优化 70% 被说成矛盾 | 检查容量约束和指标标签 |
| 杭州提升用 75% 作基准 | 改用同口径的现有仓优化 70% |
| 路线缺失仍给出结果 | 停止并补齐 Route Matrix |
| Agent 给出 Tool 中没有的数字 | 检查调用轨迹和失败规则 |
| 90% 目标被称为全球最优 | 把结论限定在给定候选点 |
| 成本被当成完整投资回报 | 列出合同未包含的成本项 |

## 你在本篇实际完成了什么

你真实验证的是一个任务中的完整能力链：

```text
Data MCP
  → Dataset Resource
  → Network Planning MCP
  → 方案与验证结果
```

两个 MCP Server 形成代码职责边界，但不自动变成两个有独立上下文的 Agent。你已经
学会怎样设计 Data Agent、Network Planning Agent 和 Supervisor 的职责；当前练习
没有把单任务调用包装成真实多 Agent 运行。

## 完成标志

- [ ] 能人工解释 100、90、55 三个数据事实；
- [ ] Dataset、Snapshot、Route Matrix 和最终结果都通过验证；
- [ ] 能解释历史履约、当前关系和现有仓优化的差异；
- [ ] 杭州方案只与同口径 70% baseline 比较；
- [ ] 90% 目标返回一个杭州候选仓和 95% 覆盖率；
- [ ] 决策摘要列出了数据警告、成本范围和下一步验证；
- [ ] 能解释 Data Agent、Network Planning Agent 和 Supervisor 为什么分工；
- [ ] 能说明当前单 Thread 能力链与真实多 Agent 轨迹的差别。
