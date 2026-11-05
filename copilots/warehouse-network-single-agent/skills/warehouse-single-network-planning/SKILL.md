---
name: warehouse-single-network-planning
description: 单 Agent 仓网规划的少见分支：真实导航、上传路线、指定单位成本、上传报价和脚本核算。
metadata:
  short-description: 单 Agent 仓网规划补充
---

# 单 Agent 仓网规划补充

常见的基线、单仓变化和选址步骤在 `$warehouse-single-agent` 中。本文件只处理用户在选项卡中选择的少见分支，以及用户明确要求的脚本核算。路线参数只来自数据或用户的选择：不得自行填写绕路系数、平均速度或导航费用。

## 路线选“真实导航”

1. `create_navigation_matrix_request`：与原来的 `prepare_route_matrix` 用同一个 `warehouse_scope`，`output_relative_path` 写成 `outputs/warehouse-network/requests/` 下的新 `.json` 文件。
2. `execute_navigation_matrix`（地图服务 `map_utils`）：`navigation_request_relative_path` 用上一步写出的文件，`output_relative_path` 写成同一目录下另一个新 `.json` 文件。它会产生外部调用费用，需要用户批准。
3. `import_navigation_matrix`：`navigation_matrix_relative_path` 用上一步写出的文件。返回的路线结果替代 `prepare_route_matrix` 的结果，继续原流程。

## 路线选“上传路线”

停止并告诉用户需要上传覆盖当前仓库范围的完整路线。用户上传后从数据准备重新开始，再用 `route_method="provided"` 调用 `prepare_route_matrix`。

## 成本选“指定单位成本”

请用户给出每一层（干线、末端）每单位需求的成本和币种，然后调用 `plan_cost_matrix`，`cost_policy={"kind": "explicit", "rules": [...]}`，每条规则写明层级、单位成本和币种。

## 成本选“上传报价”

停止并告诉用户需要上传覆盖当前仓库范围的完整报价，上传后从数据准备重新开始。

## 用户明确要求脚本核算报价均值

只有用户明确要求时才做：

1. 用 shell 或 Python 读取准确的 `prepared_input_relative_path`，不读取原始文件。
2. 在 `outputs/warehouse-network/calculations/` 下新建可复用的 `.py` 脚本，由它按 `warehouse_quote_mean_calculation.v1` 写出新的 `.json` 结果；不覆盖已有文件。
3. 调用 `plan_cost_matrix`，`cost_policy={"kind": "observed_quote_mean"}`，`quote_mean_evidence_relative_path` 用该 `.json` 路径，由工具复核。

## 选项卡

工具返回 `needs_input` 时，按 `$warehouse-single-agent` 中的选项卡规则询问并等待；用户回答后按所选分支继续，不要在用户回答前重复调用同一工具。
