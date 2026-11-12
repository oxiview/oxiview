# 像素快照

`oxiview-testing` 是 M-1 的测试 harness，只处理程序生成或调用方提供的 RGBA8 图像，不依赖窗口或渲染器。调用方在 dev-dependencies 中继承 `oxiview-testing.workspace = true`。

```rust
use std::path::Path;
use oxiview_testing::{CompareConfig, RgbaImage, SnapshotHarness};

// 示例调用方位于 <workspace>/crates/<crate>；其他布局请显式提供 workspace 根。
let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
let workspace_root = crate_root.parent().expect("crates").parent().expect("workspace");
let harness = SnapshotHarness::new(workspace_root, crate_root, env!("CARGO_PKG_NAME"))?;
let image = RgbaImage::new(1, 1, vec![255, 0, 0, 255])?;
harness.assert_snapshot("red", &image, &CompareConfig::default())?;
# Ok::<(), oxiview_testing::SnapshotError>(())
```

两个根目录必须是已存在的绝对目录，不依赖进程当前目录。基准位于调用方的 `tests/snapshots/<name>.png`，要求 RGBA8 PNG，直接提交 Git。名称仅允许 ASCII 字母、数字、连字符和下划线，`all` 为保留字。同一 crate 中每个测试使用不同的快照名，避免并行测试互相覆盖产物；测试与 accept 不应同时运行。

默认要求完全一致。`channel_tolerance` 为 0–255，检查全部四个通道，差值等于容差时通过。`allowed_diff_ratio` 为 0–1 的有限数：任一通道超限即计一个差异像素，比例等于上限时通过。尺寸不同始终失败；指标按两图包围矩形计算，缺失像素计差异、最大通道差记为 255。差异图中容差内像素淡化、差异像素不透明品红高亮。缺失基准时整图高亮，差异指标为空。

失败写入 `<workspace>/target/xtask/snapshots/<crate>/<name>/actual.png`、`diff.png`、`pending.json`。元数据记录调用方相对根目录、名称、原因和指标。测试永远不会写入基准；成功重跑会清理旧 pending，损坏的 PNG 则直接报编解码错误，不作为缺失基准接受。

1. 执行 `cargo xtask snapshot --review`，查看名称、原因、指标及文件路径，并人工查看 actual 和 diff。
2. 确认变化符合预期后运行 `cargo xtask snapshot --accept <name>`，或用 `<crate>/<name>` 消除同名歧义。`--accept all` 接受全部待处理项。
3. 命令明确输出创建或覆盖的基准路径，并删除对应 pending；actual/diff 留在 target 便于回查。重新运行测试并审查 Git 中的基准变更。

`snapshot` 不加入 `verify`，因此 verify 不接受快照。快照测试失败时，test 汇总会提示运行 review。命令遵循 xtask 的报告约定，写入 `target/xtask/report.json` 与 `snapshot.log`；`--json` 输出报告，审阅详情见报告引用的日志。接受过程逐项执行，文件系统错误时可能已有部分基准更新，应查看基准变更及剩余 pending 后重试。

新增依赖 [`png 0.18.1`](https://github.com/image-rs/image-png)、[`thiserror 2.0.21`](https://github.com/dtolnay/thiserror) 均为持续维护的纯 Rust 项目，许可证 MIT OR Apache-2.0；只启用默认稳定功能，不引入完整 image 库。serde/serde_json 沿用 workspace 版本，依赖审查统一由 `cargo xtask deps` 执行。
