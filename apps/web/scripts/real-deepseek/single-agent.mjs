// Single-agent business scenario: natural request plus typed acceptance helper.

export const taskPrompt = [
  "请根据已上传的数据，在保留现有仓的前提下计算仓库分布方案，使 12 小时需求量加权达标率至少达到 90%。",
  "路线统一使用 haversine，绕路系数为 1.2，平均速度为 42 kph；请从不新增仓开始，找出满足目标的最少新增仓方案，并在同一仓数下选择运输成本较低的方案。",
  "成本请使用现有完整报价数据计算报价均值，不要只根据预览行估算；本次明确要求用脚本完成这个均值计算，并说明计算依据和结果。",
  "请由当前 Agent 独立完成分析，给出仓数、12 小时覆盖率和成本假设。",
].join("\n");

export function isMinimumFeasibleSolution(result, serviceTargetHours) {
  const coverage = result?.solution?.coverage;
  const metric = Array.isArray(coverage)
    ? coverage.find((entry) => entry?.target_hours === serviceTargetHours)
    : undefined;
  return (
    result?.state === "ready" &&
    ["optimal", "feasible"].includes(result?.status) &&
    Number.isInteger(result?.selected_number_to_open) &&
    typeof result?.minimum_number_to_open_proven === "boolean" &&
    Array.isArray(result?.solver_stages) &&
    result.solver_stages.length <= 2 &&
    result.solver_stages.some((stage) => stage?.kind === "minimum_openings") &&
    result.solver_stages.some((stage) => stage?.kind === "minimum_cost") &&
    typeof metric?.demand_weighted_coverage_rate === "number" &&
    metric.demand_weighted_coverage_rate >= 0.9
  );
}

export function hasNoChildCollaboration({ collaboration, nativeToolNames }) {
  return (
    collaboration.length === 0 &&
    !nativeToolNames.some((name) => /spawn_agent|send_input|wait_agent|resume_agent/.test(name))
  );
}

export function validateGeneratedFiles({ generatedWorkspaceFiles, calculationFiles }) {
  const valid =
    generatedWorkspaceFiles.some((relativePath) =>
      relativePath.startsWith("outputs/warehouse-network/prepared/"),
    ) &&
    calculationFiles.some((relativePath) => relativePath.endsWith(".py")) &&
    calculationFiles.some((relativePath) => relativePath.endsWith(".json")) &&
    generatedWorkspaceFiles.every((relativePath) =>
      relativePath.startsWith("outputs/warehouse-network/"),
    );
  return valid
    ? { valid: true }
    : {
        code: "single_agent_generated_output_scope_invalid",
        details: { generatedWorkspaceFiles: generatedWorkspaceFiles.slice(0, 60) },
      };
}

export function validateQuoteMeanEvidence({
  calculation,
  costResult,
  calculationJsonPath,
  calculationFiles,
  canonicalJson,
}) {
  const expectedMeans = {
    last_mile: 1_968_472.727273,
    linehaul: 1_252_333.333333,
  };
  const expectedEvidenceKeys = [
    "considered_quote_count",
    "formula",
    "ignored_quote_count",
    "input_identity",
    "method",
    "prepared_input_relative_path",
    "rules",
    "schema_version",
    "tool_version",
    "total_quote_count",
  ];
  const evidenceKeys = Object.keys(calculation ?? {}).sort();
  const valid =
    JSON.stringify(evidenceKeys) === JSON.stringify(expectedEvidenceKeys) &&
    calculation?.schema_version === "warehouse_quote_mean_calculation.v1" &&
    calculation?.method === "observed_quote_mean" &&
    calculation?.tool_version === "observed-quote-mean.v1" &&
    calculation?.formula === "arithmetic_mean(price_per_vehicle / vehicle_capacity)" &&
    typeof calculation?.prepared_input_relative_path === "string" &&
    calculation.prepared_input_relative_path.startsWith("outputs/warehouse-network/prepared/") &&
    /^[a-f0-9]{64}$/.test(calculation?.input_identity?.content_sha256 ?? "") &&
    calculation?.total_quote_count === 580 &&
    calculation?.considered_quote_count === 580 &&
    calculation?.ignored_quote_count === 0 &&
    Array.isArray(calculation?.rules) &&
    calculation.rules.length === 2 &&
    Object.entries(expectedMeans).every(([layer, expectedMean]) => {
      const rule = calculation.rules.find((entry) => entry?.layer === layer);
      return (
        rule?.currency === "IDR" &&
        Number.isInteger(rule?.quote_count) &&
        rule.quote_count === (layer === "last_mile" ? 550 : 30) &&
        Number.isFinite(rule?.mean_cost_per_demand_unit) &&
        Math.abs(rule.mean_cost_per_demand_unit - expectedMean) < 1e-3
      );
    }) &&
    calculationFiles.includes(calculationJsonPath) &&
    calculationJsonPath.endsWith(".json") &&
    costResult?.cost_rule_evidence_path === calculationJsonPath &&
    JSON.stringify(canonicalJson(costResult?.cost_rule_evidence)) ===
      JSON.stringify(canonicalJson(calculation));
  return valid
    ? { valid: true }
    : {
        code: "single_agent_calculation_policy_mismatch",
        details: {
          evidence_keys: evidenceKeys,
          evidence_rules: Array.isArray(calculation?.rules)
            ? calculation.rules.slice(0, 2)
            : undefined,
        },
      };
}
