# 仓网 Copilot 新手教程

这份教程让第一次使用者在 Web 中完成一条真实的仓网分析：上传文件，计算时效，比较新增仓，再找出满足目标的最少新增仓。

它不要求你学习 Agent、工具调用、文件路径或计算实现。只需要选择正确的 Copilot，回答确实需要业务判断的输入卡片，并阅读最终的数字和地图。

从 [仓网 Copilot 新手教程](tutorials/README.md) 开始。

## 推荐学习顺序

| 顺序 | 教程 | 完成后你能做什么 |
| --- | --- | --- |
| 1 | [10 分钟跑通完整示例](tutorials/indonesia-network-quickstart.md) | 在新 Workspace 中完成三次提问和三张地图 |
| 2 | [认识要上传的数据](tutorials/indonesia-network-01-data.md) | 用自己的 CSV、Excel 或 JSON 替换示例文件 |
| 3 | [查看 12 小时时效和地图](tutorials/indonesia-network-02-service-baseline.md) | 分别看需求量加权和城市达标率 |
| 4 | [评估新增 Balikpapan 仓](tutorials/indonesia-network-03-two-level-cost.md) | 比较一个明确新增仓前后的影响 |
| 5 | [找出达到 90% 的最少新增仓](tutorials/indonesia-network-04-optimization-map.md) | 得到最少新增仓、选址和地图 |

## 两个使用原则

1. 在同一个任务里连续追问。已准备的数据在没有变化时可以继续使用，结果也更容易比较。
2. 只在输入卡片要求时补充信息。系统会自己检查数据；你无需用技术术语描述文件或指定内部步骤。

## 进一步阅读

- [仓网 Copilot 是什么](tutorials/supply-chain-agent-tutorial.md)：用业务语言介绍它能做什么。
- [审批与故障恢复](tutorials/approvals-and-recovery.md)：任务等待、审批或失败时怎么处理。
- [Copilot 开发者中心](developers/copilot/README.md)：开发与维护 Copilot 的唯一入口。
- [开发仓网 Copilot](developers/copilot/warehouse-copilot.md)：共享领域 Tool、单 Agent 仓网包、Role、
  delivery 与三轮 Web 地图验收。
