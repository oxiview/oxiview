---
name: adr-glossary
description: 判断是否需要 ADR、创建或替代 ADR、维护术语表，或发现用户术语与项目定义冲突时使用。
---

# ADR 与术语

## 决策记录

按[Git 公约](../../../docs/conventions/git-workflow.md)判断：难以逆转、缺少上下文会令人意外、确有权衡。三条均满足才写 ADR，否则在代码或变更说明中解释。

1. 阅读 [ADR 索引](../../../docs/adr/README.md)及相关决策。
2. 扫描 `docs/adr/` 全部编号文件，取最大编号加一、补足四位，创建 `NNNN-英文短名.md`。不要按文件数或仅凭索引猜编号，协作时先协调编号。
3. 沿用现有 `status` frontmatter，正文简短写清决策、背景与取舍；按需添加 Status / Considered Options / Consequences。
4. 替代旧 ADR 时保留历史理由，将旧记录设为 `status: superseded`，在 Status 段链接新 ADR；新记录链接旧 ADR 并说明替代范围，决定获接受后用 `status: accepted`。
5. 同次更新 `docs/adr/README.md` 的编号、标题、链接与维度，运行 `cargo xtask verify` 检查链接。

## 术语表

先读 [GLOSSARY.md](../../../GLOSSARY.md)。用户使用冲突术语时立即指出规范词及含义差异，再继续任务，不静默沿用歧义。

新增条目沿用所属分组：

```markdown
**中文术语（English term）**：
只定义它是什么。
_Avoid_: 避免用词、易混词
```

定义不写实现细节、操作步骤或设计理由，这些进入架构、公约或 ADR。纯英文术语沿用既有格式，没有避免用词时可省略 `_Avoid_`。改名时同步相关文档与注释，保留历史 ADR 的决策理由。
