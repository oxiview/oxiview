---
status: accepted
---

# 验证：只做本地验证，`cargo xtask` 是唯一入口，不建 CI

目前没有 CI 的资源预算，所有验证都在本地（macOS 开发机）完成。`cargo xtask` 是唯一的验证入口：格式、lint、测试、像素快照、基准与性能指标、依赖审查、依赖方向检查，都通过 xtask 子命令完成。人和 agent 跑的是同一条命令。凡是路线图和其他 ADR 中写到“进入 CI”的地方，在 CI 出现之前一律理解为“进入 `cargo xtask verify`”。

## Consequences

- 性能硬指标（ADR-0001）的基线依赖本地硬件，基线文件必须记录机器型号与系统版本，跨机器的比较只作参考。
- 没有服务器强制把关，所以“提交前必须通过 `cargo xtask verify`”写进公约（AGENTS.md），可以加上本地 git pre-push hook 辅助执行。
- 将来引入 CI 时，CI 只需调用同一个 xtask 入口，不在 CI 配置里写验证逻辑。
