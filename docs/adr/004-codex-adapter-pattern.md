# ADR-004: Codex Adapter 抽象模式

**状态**: 已接受  
**日期**: 2026-07-12  
**关联**: ADR-001, ADR-003

## 背景

平台需要与 Codex app-server 通信来启动 Thread、执行 Task、观察事件和处理审批。在开发环境中, 连接真实的 app-server 进程引入复杂性和依赖。需要一个抽象层允许离线开发和测试。

## 决策

使用 **Trait-based Codex Adapter** 模式:

```rust
#[async_trait]
pub trait CodexAdapter: Send + Sync {
    async fn health(&self) -> Result<HealthStatus, AdapterError>;
    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AdapterError>;
    async fn subscribe_events(
        &self,
        sender: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<(), AdapterError>;
}
```

两个实现:

- **RealCodexAdapter**: 当前直接复用原生 Profile Host 的 app-server stdin/stdout 连接、Manifest 协商和有界事件流，不再依赖 Tauri daemon；多用户 Profile Registry 和类型化服务接口仍在迁移中
- **FakeCodexAdapter**: 内存中模拟 app-server 行为, 用于开发和集成测试

## 理由

- 开发环境无需启动 Codex 二进制
- 单元测试可以在 CI 中运行 (无需 Codex 环境)
- 真实和 mock 之间使用同一接口
- 未来可以增加 HTTP 传输 (Remote Codex Adapter)

## 替代方案

- **No abstraction**: 直接调用 app-server 进程, 测试需要真实环境
- **Mock library**: mockall 等自动 mock, 但失去对模拟行为的控制力

## 影响

- 正向: 离线开发可行; 集成测试不需要真实 app-server; 接口变更影响在一个地方
- 负向: trait 需要随协议演进维护; 增加一层间接调用

## 当前约束

`rpc` 是内部迁移接口，不能原样成为浏览器公共 API。原生 Profile Host
已经落地，下一步必须将 Adapter 收窄为 Thread/Turn/Approval/Capability
等类型化内部操作，并在调用前完成 Profile、Thread 和 Workspace 归属校验。
