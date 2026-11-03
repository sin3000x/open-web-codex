# Supply Chain Network Planner

这个 Tool source 提供通用仓网规划能力。Codex Runtime 负责 Thread、Turn、Agent、Skill、MCP
和模型执行；SDK generic provisioner 负责依赖环境、构建、缓存、超时和 transport projection；
本包只负责直接依赖声明、Data/Network 领域工具；Data inspection 是 inline typed 结果，Network
规划中间结果仍由唯一 provider ResourceStore 持久化。

## MCP 入口

`runtime.toml` 声明同一 Python project 中的两个 module entry；它不包含安装命令，Tool source
也不提供 launcher 或 source `.mcp.json`：

| Server | 责任 |
| --- | --- |
| `supply_chain_data` | 发现和检查授权 Workspace 的 CSV/JSON/XLSX，发布来源画像、映射、标准化输入和行政区结果，并在准备好的输入上回答只读事实查询 |
| `supply_chain` | 构建路线/成本矩阵，计算覆盖、成本、场景、p-median、服务约束选址、比较地图和报告 |

`network/server.py` 只负责创建 FastMCP、按固定顺序注册一个 Resource template 与 12 个公开 Tool、
解析 transport 和初始化 Runtime。Network Tool 的 owner 分别位于 `tool_runtime.py`、`route_tools.py`、
`cost_tools.py`、`analysis_tools.py`、`facility_tools.py` 与 `delivery_tools.py`；它们共享同一个
Workspace/Profile-scoped ResourceStore accessor，不各自创建 MCP 或业务缓存。
没有独立的 Indonesia 或 Demo MCP。国家由用户问题确定，行政区能力属于 `supply_chain_data`；地图由 Network Agent 使用 Planner 的确定性地图工具生成。示例数据只能由用户显式放入 Workspace，工具不会在失败时自动回退到 Demo fixture。

## 当前网络工具

Network Agent 的主要工具是：

```text
prepare_route_matrix
create_navigation_matrix_request
import_navigation_matrix
plan_cost_matrix
prepare_network_distribution_map
evaluate_network_baseline
assess_facility_change
solve_p_median
compare_network_scenarios
prepare_network_comparison_map
publish_network_planning_report
```

Data Tool 先以 `workspace_source_profile.v2` inline `source_profile`、exact source units、`inspection_identity` 和 `inspected_relative_paths` 返回有界检查结果，再重新检查完整文件并以 create-new 语义写入用户可见的 `outputs/warehouse-network/prepared/` Workspace JSON；Data/Network Agent 只交接精确相对路径、内容身份、roles/role_counts 和有界 warnings。检查只接受原始来源；已准备输入由 `resolve_prepared_network_input` 按本次所需 roles 的规范化记录比较复用：记录相同即 `prepared_ready` 直接交接，记录不同才 `prepared_selection_required` 请求一次选择，没有 fresh 文件返回 `prepared_unavailable` 后再检查原始来源；`source_changed` 最多重检一次。Data 不注册 Resource template，也不要求读取 source profile Resource。导航请求只写入 `outputs/warehouse-network/requests/`，单 Agent 经 Skill 授权的有界全量统计脚本与结果只写入 `outputs/warehouse-network/calculations/`，最终地图/报告文件只写入 `outputs/warehouse-network/deliverables/`，所有输入源保持原位。路线、成本、方案与比较等 Network 计算结果写入 Resource Store，并都绑定同一输入身份。Agent 不传原始文件、整张矩阵或完整工具结果。缺失数据、矩阵不完整、求解器不可用和超时都是显式状态；不会自动加载 Mock、填零或切换旧实现。

`query_prepared_network_input` 是 Data 的只读事实查询入口：对一个准备好的输入中某个数据集
（warehouses、demand_cities、current_assignments、route_quotes、provided_route_facts）的全部记录做
typed 过滤、最多两个字段分组、sum/min/max/mean 以及最多 100 行的排序列表，返回
`prepared_network_query.v1` 的精确 `record_count`、`matched_count` 和分组计数。字段按记录模型分为
text/number/bool，非法字段、类型不符或范围比较作用于非数值字段时返回带原因的错误；所选原始来源已变化时
返回 `prepared_input_stale`，不返回过期事实。“有几个中心仓”这类问题由它回答，不读取预览、不跑脚本。

Data Tool 会把用户输入中完整的起点、终点、距离、时长与来源方法保存在
`prepared_network_input.v2`；Network Tool 可以按分析范围把这些事实物化为
`route_matrix.v3`。矩阵自身持有 discriminated `warehouse_scope`（existing_only、精确 candidate_ids 或 all_warehouses）与规范化 `warehouse_ids`，验证器按同一 scope 校验，不会因标准化资源
同时包含未参与本次分析的候选仓而要求 Data 重新发布资源；也无需让模型重读文件或重新估算。基线和比较结果同时返回按城市数量与按需求量
加权的覆盖指标，并以 typed Resource 支持实际基线、优化基线或场景之间的比较；模型只负责解释，
不自行汇总这些数值。

`plan_cost_matrix` 的 `cost_policy` 是当前唯一成本 fallback 选择：`kind=explicit` 接受显式分层数值规则；`kind=observed_quote_mean` 由 Planner 直接读取完整 prepared input，对当前所需层的全部标准化报价按 `price_per_vehicle / vehicle_capacity` 求算术均值，并把 `warehouse_quote_mean_calculation.v1` 的 prepared input identity、完整报价总数、分层报价数、币种、公式、均值和 Tool 版本作为 bounded provenance 返回并绑定到 `cost_matrix.v3`。用户明确要求脚本时，必须把同一 typed evidence 写入 calculations 目录并传 `quote_mean_evidence_relative_path`；Planner 会从完整输入重算并校验，脚本 JSON 不是第二份业务真相。该计算不读取 preview，也不把完整报价行送入模型上下文。

`solve_p_median` 的 `opening_policy` 支持 `exact` 和一次有界的 `minimum_feasible` 两阶段求解。后者在一个总 `time_limit_seconds` 内先最小化新增仓数，再在该仓数下最小化成本，返回最多两个 typed `solver_stages`、`selected_number_to_open` 和最终 coverage，避免由 Agent 循环调用多个求解 Tool。报价均值外推且距离成本为零只是敏感性方案；没有仓租、建设、容量或吞吐成本时，结果不得称为经济意义上的全局最优。

地图数据合同发布仓库、需求城市、分配关系、逐城市距离、时长、成本以及相对用户所选服务目标的
`attained/missed/unassigned` 状态，不包含标题、颜色、大小、标签、悬浮信息或图例。
`prepare_network_coverage_map` 要求确切 `service_target_hours` 并验证该目标存在于计算结果；高层
`map_utils/create_network_map_card` 只按这些 Planner 事实生成默认仓网图层和图例，不重新计算 SLA。

`assess_facility_change` 接受精确的标准化输入、路线、可选成本以及任意合法的
baseline/scenario/facility-location `before_ref`，以引用中的活动仓集合为起点，一次完成增仓、关仓或迁仓后的分配求解和前后比较。
它发布完整的 `network_scenario.v2` 与绑定输入、前后方案和比较结果的
`network_plan_comparison.v2` Resource，同时只把
有界的仓库变化、成本、两种覆盖率和最多 10 个重点受影响/重分配城市返回给 Agent；
`compare_network_scenarios` 与 `assess_facility_change` 均发布同一 before/after 合同。
单仓增加、关闭或搬迁统一由 `assess_facility_change` 完成计算与比较；不再向 Agent 暴露重复的独立场景入口。

## Tool 审批

Data Role 的发现/检查 Tool 是只读能力，准备/地理补全 Tool 是只允许在上述 package-owned 输出目录 create-new 的有界本地写入能力，可在精确 allowlist 内预批准。
Network/standalone Plugin 以 `prompt` 为默认，只对路线准备、baseline、scenario、
p-median、comparison 和卡片数据准备等无外部副作用的 Tool 配置逐项预批准。对话内地图卡片
复用 `map_utils/create_map_card` 或 `revise_map_card`，当空间分布、覆盖关系、仓库变动或城市重分配有助于理解时可由
Skill 自动使用，不创建 Workspace 文件。最终 map 文件与 Markdown report Tool 会以 create-new
语义写 Workspace 文件，因此继续请求 official approval。地图导航和距离矩阵属于 `map_utils`
的外部、可能计费操作，也不能由本包预批准。Tool annotations 描述 provider
事实，Role/Plugin policy 决定当前 Agent 的精确预批准面；两者都不改变全局 Runtime
`approvalPolicy`。

仓库—需求城市对应关系、距离、时长和成本是结构化计算结果；业务结果简报是独立的 Markdown
交付。`publish_network_planning_report` 通过带判别字段的 `report_input` 分别接受单一 baseline
评估或单一 `plan_comparison_ref`，从经过验证且来源一致的 typed 结果确定性生成中文 `.md`，同时返回
一个指向该 Workspace 文件的安全相对链接。对话只概括关键结论并展示链接，不重复插入整份简报；完整对应明细不重复塞进简报；需要 Excel 时必须使用真实
表格导出 Tool，当前没有该 Tool 就显式报告能力缺口，不能用 JSON 或改扩展名冒充 Excel。

## 6.0 能力包

| Package | 责任 |
| --- | --- |
| `enterprise-data-agent@6.0.0` | 来源发现、字段映射、行政区解析、坐标和边界校验、按需标准化 |
| `enterprise-network-planning-agent@6.0.0` | 需求定义、路线/成本口径、覆盖、成本、场景、选址和地图/报告 |
| `enterprise-supervisor-copilot@6.0.0` | 动态协调 Data 和 Network，不写死国家、阶段数量或调用顺序 |

Supervisor 不绑定 Visualization Agent。child 只把 typed `needs_input` 交给 Root；只有 Root 调用 Runtime 官方 `request_user_input`，平台把该请求投影到 Run 输入队列。普通 Assistant 文字不能代替输入卡片。

## 印尼教程 Fixture

已验证数据位于：

```text
examples/indonesia-network/base/
examples/indonesia-network/current-coverage-extension/
```

基础 fixture 包含 50 个需求城市、5 个中心仓、6 个 cross-docking 仓、550 条末端报价和 30 条干线报价。需求量是 `ceil(population / 1000)`；城市点位经过 geoBoundaries ADM2 point-in-polygon 校验；报价与距离的 Spearman 相关系数不低于 0.85。`current-coverage-extension` 只在用户需要实际当前方案时使用。

教程生成器会保存来源 URL、获取日期、许可证、原始 hash 和生成器版本：

```bash
python3 tools/warehouse-network-planner/scripts/generate_indonesia_tutorial_data.py \
  --network-fixture \
  --download-missing-sources
```

生成器只在显式给出 `--download-missing-sources` 时下载经过 SHA-256 锁定的原始边界缓存；
这些可重建的大文件和生成 release 不纳入版本库。生成器不访问模型、不修改 Runtime，也不会把
fixture 作为空 Workspace 的自动回退。

## 开发和验证

```bash
python3 -m pip install -e packages/copilot-provider-sdk -e packages/copilot-sdk
python3 -m copilot_sdk prepare copilots/warehouse-network \
  --output-root "$PWD/.local/open-web-codex/copilot-environment"
python3 -m pip install -e 'tools/warehouse-network-planner[dev]'
python3 -m pytest tools/warehouse-network-planner/tests -q
```

第一条命令验证平台实际使用的通用环境编译合同；领域单元测试仍可在开发者已准备的 Python
环境运行。Platform local startup 同样只调用一次 `copilot prepare`，不会按 supply-chain 名称
分支，也不会在 Runtime launch 或用户对话期间安装依赖。

真实 stdio smoke 只在明确设置 `RUN_REAL_STDIO_SMOKE=1` 时启动 Data、Network 和 maps 三个 stdio server 实例。任何 MCP Tool 都通过 Runtime 的 MCP 调用路径执行，禁止通过 shell `source`、目录遍历或手工拼接 Resource URI 调用。
