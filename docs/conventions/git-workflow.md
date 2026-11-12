# Git 工作流

单人阶段直接在 main 上小步提交，每次提交可独立审查并保持验证通过；较大改动使用短期分支，完成后及时合回，避免长期分叉。仅实现当前获准的步骤，不顺手提前实现后续里程碑。

## 提交格式

使用 Conventional Commits：`type(scope): 描述`，scope 可省略，使用 crate 短名（去掉 `oxiview-`，如 `testing`、`render-cpu`；工程工具用 `xtask`）。type 为英文，描述可中文。

| type | 用途 |
| --- | --- |
| feat | 新能力 |
| fix | 缺陷修复 |
| docs | 文档 |
| refactor | 不改变行为的结构调整 |
| perf | 性能改进 |
| test | 测试 |
| build | 构建与依赖 |
| chore | 工程维护 |

例如 `docs: 补齐工程公约`、`test(xtask): 覆盖文档链接失效场景`。提交前必须执行 `cargo xtask verify` 并通过；性能相关变更另按[基准公约](testing-performance.md)验证。审查 `target/xtask/report.json` 和变更内容，报告不入库。快照与基线变更必须经过人工确认，不能只看命令成功。

## 可选的本地 hook

显式运行 `cargo xtask install-hooks` 安装 `pre-push` hook；检出仓库、构建和 verify 都不会自动安装。hook 使用 POSIX sh，在当前仓库根执行 `cargo xtask verify`，失败时以相同退出码阻止本次推送。

安装通过 `git rev-parse --git-path hooks` 定位目录，遵循 worktree 与 `core.hooksPath` 配置；共享该目录的检出也会受到影响。重复安装相同内容是幂等的，并在 Unix 上补齐执行权限。已有不同内容时拒绝覆盖；显式使用 `cargo xtask install-hooks --force` 会先备份为同目录的 `pre-push.backup`，已有备份时拒绝继续，需自行处理备份。非普通文件（包括软链）拒绝修改。

`cargo xtask install-hooks --uninstall` 仅删除与本命令生成内容完全匹配的 hook；内容被修改时拒绝删除，不存在时无需操作。备份保留，恢复原 hook 由使用者自行完成；`--force` 与 `--uninstall` 不能同时使用。本地 hook 是辅助约束，不能替代提交前验证与人工审查。

## ADR 与术语

采用 domain-modeling 的三条判断标准，**同时满足**时必须写 ADR：

1. 难以逆转：日后改变决定有实质成本。
2. 缺少上下文会令人意外：未来读者会问“为什么这样做”。
3. 确有权衡：比较过真实可行的替代方案，并因明确原因作出选择。

不满足时在代码或变更说明中解释即可。ADR 使用 `docs/adr/NNNN-英文短名.md`，取现有最大编号加一并补足四位，状态和结构沿用现有记录；同次变更同步 [README 索引](../adr/README.md)。多人或多 agent 协作时先协调编号，避免碰撞；替代决策以新 ADR 说明关系，不静默重写历史理由。规范术语取自 [GLOSSARY.md](../../GLOSSARY.md)。

## 协作与远端

多 agent 共同贡献同一提交时，由协调者汇总验证并提交，在提交消息正文后空一行，按实际贡献添加 `Co-Authored-By: 名称 <邮箱>` trailer；每位共同作者一行，使用其已确认的署名，不虚构身份或邮箱。不把仅运行工具或旁观视为共同作者。

没有提交权限的 agent 提供变更和建议提交消息，由协调者执行提交。除非所有者明确要求，不推送到远端，不因分支已完成而自动推送。
