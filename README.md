# oxiview

只面向 macOS、Windows、Linux 原生平台的 Rust 自绘 UI 框架。

目前处于 M-1「工程规范」里程碑，尚未实现框架功能。
工具链固定为 Rust stable 1.97.0，使用 edition 2024。

统一的本地验证入口：

```sh
cargo xtask verify
```

详细范围见 [M-1 计划](docs/plans/m-1-engineering-baseline.md)，
术语见 [GLOSSARY.md](GLOSSARY.md)。

许可证：MIT OR Apache-2.0，版权人 The oxiview Authors。

`verify` 先检查 rustc 1.97 stable 和 cargo-deny，再依次执行格式检查、
Clippy 和测试，检查失败后继续汇总。S3–S8 的检查尚未接入。

其他入口：`cargo xtask fmt [--check]`、`lint`、`test`、`doctor`。
所有子命令支持 `--json`，此时 stdout 只输出 JSON。
每次运行覆盖 `target/xtask/report.json`，子进程日志也保存在该目录。
报告每项包含 `name`、`status`（pass/fail/error）、`duration_ms`、
`failure_summary` 和 `artifacts`（相对仓库根目录）。
退出码：0 全部通过，1 检查失败，2 环境或 xtask 自身错误。
doctor 仅提示缺失工具的安装命令，不自动安装。

当前没有框架 crate；Cargo 不接受未匹配的成员通配符，
创建首个框架 crate 时需将 `"crates/*"` 加回 workspace members。
