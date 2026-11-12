---
name: verify
description: 日常改动、渲染或性能变更完成，或验证、快照、性能基线失败时使用，选择 xtask 命令并解读报告。
---

# 验证

所有命令从仓库根目录运行，只在本地验证。

## 选择命令

- 日常迭代按需要运行 `cargo xtask fmt`、`cargo xtask lint`、`cargo xtask test`；依赖变更运行 `cargo xtask deps`。
- 交付前运行 `cargo xtask verify`，包括 doctor、fmt、lint、test、deps、docs；docs 没有独立子命令。
- 涉及渲染时运行 `cargo xtask test` 并检查相关像素快照，再运行 verify。M-1 只有程序生成图像的 harness 自测，不能宣称已验证真实渲染。
- 性能相关改动运行 `cargo xtask bench` 或 `cargo xtask verify --full`；普通 verify 不跑基准。测量要求见[基准公约](../../../docs/conventions/testing-performance.md)。
- 环境问题运行 `cargo xtask doctor`；缺工具只报告安装命令，不自动安装。参数查 `cargo xtask --help` 或 `cargo xtask help <子命令>`。

## 读取结果

每次调用覆盖 `target/xtask/report.json`，下一次运行前保留需报告的结果。读取 `checks` 内的 `name`、`status`、`duration_ms`、`failure_summary`、`artifacts`，按路径查看 `target/xtask/<检查名>.log`。`--json` 使 xtask 的 stdout 仅输出 JSON。

退出码：0 全部通过；1 检查失败；2 环境缺失、参数或 xtask 自身错误（优先于 1）。doctor 失败时 verify 提前返回，其他检查失败仍继续汇总。bench 详情在 `performance` 和 `bench-results.json`，原始测量在 `bench-1.log` 至 `bench-3.log`。

## 快照失败

1. 运行 `cargo xtask snapshot --review`，读取原因与产物路径。
2. 向人展示 `actual.png` 与 `diff.png`，说明差异；基准缺失也需要确认。
3. 非预期变化先修复代码；只有人明确确认后才运行 `cargo xtask snapshot --accept <name>`，同名时用 `<crate>/<name>`。`--accept all` 需要确认全部待处理项。
4. 重跑测试与 verify，审查基准变更。详见[像素快照公约](../../../docs/conventions/testing-snapshots.md)。

## 性能回归

先排查代码、基准输入、机器负载与工具链差异。只有人确认是预期变化才运行 `cargo xtask bench --record`，随后运行 `cargo xtask bench` 确认可复现并审查基线。首次缺少本机基线也须人确认。不得用 accept 或 record 掩盖失败。

报告实际命令、退出码、失败原因与未验证部分，不把 pending 指标写成通过。
