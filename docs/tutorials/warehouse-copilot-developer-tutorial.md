# 仓网 Copilot 开发教程（入口已迁移）

> **适合谁**：从旧链接进入的仓网开发者
> **预计时间**：1 分钟
> **前置条件**：已能运行仓库统一 Copilot CLI
> **完成结果**：进入当前单 Agent 仓网包、共享 Tool registry 的唯一教程

请继续阅读[开发仓网 Copilot](../developers/copilot/warehouse-copilot.md)。当前教程明确覆盖：

- 维护单 Agent 仓网包，每类业务问题对应一个分析 Tool；
- 复用根级 `warehouse-network-planner` 与 `warehouse-network-maps`，不复制算法；
- 精确 Role、delivery 和 registry 合同；
- 12 小时 → Balikpapan → 90% 三轮真实地图验收。

成功信号：两个仓网包分别通过 `./scripts/copilot.sh check`，并在 Web 完成三轮结构化结果与地图。
失败进入[仓网真实验收排错](../developers/copilot/testing-and-troubleshooting.md#web-与真实业务验收)。
