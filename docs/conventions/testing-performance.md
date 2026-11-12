# 基准与性能基线

`cargo xtask bench` 是 workspace 的性能验证入口。默认 `cargo xtask verify` 不运行基准；`cargo xtask verify --full` 在普通验证之后追加 bench，前面的检查失败也继续汇总（doctor 环境检查失败仍提前返回）。仅使用 Rust stable 1.97、edition 2024，不需要 nightly。

## 编写基准

默认使用 [Divan 0.1.21](https://github.com/nvzqz/divan)，该项目持续维护，许可证 MIT OR Apache-2.0，MSRV 为 1.80。本仓库精确固定版本，升级时必须重新验证输出解析。新增传递依赖主要是 divan-macros、condtype、regex-lite、terminal_size 及其系统依赖；许可证、安全公告与重复版本由 `cargo xtask deps` 审查。

依赖版本仅在根 `[workspace.dependencies]` 定义，各 crate 在 dev-dependencies 中继承 `divan.workspace = true`。按 ADR-0021，workspace 示例源码在根 `benches/compare.rs`，由 `oxiview-testing` 的 `[[bench]]` 指定路径并设置 `harness = false`。新增 target 也遵循此约定；不要另建框架 crate。

示例测量 S4 的 `compare`：256×256 与 1024×1024 的相同渐变 RGBA8 图像，包含逐像素比较及差异图分配，输入图像构造在计时之外。通过 black_box 保留真实计算，结果交给 Divan 消费。基准中可使用 expect，库代码仍禁止 unwrap/expect/panic。

## 测量与解析

xtask 顺序执行三次 `cargo bench --workspace --benches --locked`，使用优化后的 bench profile。示例每轮 100 个样本、每样本 10 次迭代；每轮读取 Divan 的 median，再取三轮中位数作为最终值（统一 ns）。这不是置信区间或统计显著性检验；运行时应保持供电、散热、编译配置与后台负载稳定，不并行运行其他性能测试。

Divan 当前提供终端统计表。解析器校验表头与列数，恢复树状名称为 `target/函数/参数`，支持 ps/ns/µs/μs/us/ms/s；要求有限正数、非零采样数，拒绝重复指标、空输出和三轮指标集合不一致。清除子进程继承的 `DIVAN_*` 覆盖项，避免过滤或 test 模式改变测量。新增基准使用不含空白的名称，不启用计数器附加行或自定义输出。格式变更必须修改解析器及测试，不能把解析失败当成通过。标准 libtest 的零基准输出忽略。

报告写入 `target/xtask/report.json`，其中 `performance.metrics` 包含各项当前值、基线值、状态与阈值，硬指标显示 `pending`；`bench-results.json` 是同一详细数据的独立产物。`bench.log` 保存判定摘要，`bench-1.log` 至 `bench-3.log` 保存原始 stdout/stderr。每次 bench 先清理这些旧产物，避免失败时引用过期测量；`--json` 只向 stdout 打印报告。

## 记录与更新

1. 关闭重负载程序，确认当前代码和基准输入符合预期。
2. **必须由人确认后**才执行 `cargo xtask bench --record`。命令会明确提示创建或覆盖 `perf/baselines/<machine-id>.json`，该文件需要审查并提交。默认 bench 和 verify 永不写基线。
3. 紧接着运行 `cargo xtask bench`，确认测量可复现；出现回归应先排查代码和环境，不要通过反复 record 隐藏失败。
4. 更新已有基线保留各指标的 `regression_percent`（默认 10，可逐项覆盖为 0–100），更新数值、机器描述和记录时间。已记录指标消失时即使 record 也失败，删除或重命名指标需要人工审查基线变更。

机器标识使用架构、型号、CPU 名称、内存字节数的固定 FNV-1a 64 位哈希，附操作系统名称。macOS 读取 `hw.model`、`machdep.cpu.brand_string`、`hw.memsize`，沙箱拒绝时使用 `system_profiler SPHardwareDataType -detailLevel mini -json` 的对应字段（mini 排除序列号和 UUID）；Windows 使用 CIM 型号/CPU/物理内存；Linux 使用 DMI 型号、`/proc/cpuinfo`、`/proc/meminfo`。不读取序列号、用户名或主机名。操作系统版本与 rustc 版本记录在描述中，不参与哈希，重复运行得到同一 ID。

硬件配置相同的机器会共享标识；不使用设备唯一隐私标识就无法保证区分它们。内存升级会生成新 ID；Linux 的 MemTotal 是内核可用内存，内核/启动配置变化也可能生成新 ID。不支持的探测环境明确报错，不退化为 unknown ID。Windows/Linux 实现尚需在对应原生平台实测。

文件使用 schema_version=1，时间为 UTC Unix 秒，指标记录 value、unit、direction（lower 越小越好、higher 越大越好）、regression_percent、target。本步骤的微基准均为 lower；硬指标 value=null 表示未测量，不等于零。

## 判定规则

- lower：当前值减基线值大于 `基线值 × 阈值 / 100` 才失败；higher 使用基线值减当前值。恰好等于阈值通过，改善通过；基线为零时不允许向坏方向增加。
- 本机无基线、新指标无基线或已记录指标未测量：失败，退出码 1。无基线时提示人工确认后使用 `--record`，避免把未经比较的结果视作已验证。
- 回归及基准子进程非零退出：失败，退出码 1。环境、文件或解析错误：退出码 2，与既有 xtask 约定一致。
- 其他机器基线只在日志中显示原始内容，不参与校验和判定。当前机器的系统与工具链描述也显示在日志中；跨系统/工具链更新应人工评估是否重新记录。

## 硬指标接入计划

| 指标 | 目标 | 当前状态 | 接入时机 |
| --- | --- | --- | --- |
| `input_to_pixel_ms` | 8.0 ms | pending | M1 接入输入到呈现的真实测量 |
| `idle_cpu_percent` | 0.0 % | pending | M1 接入空闲帧策略及 CPU 测量 |
| `list_100k_scroll_dropped_frames` | 0 frames | pending | M7 接入 10 万行列表滚动测量 |

三个指标仅预留目标，不伪造测量、不参与本步骤的通过/失败判定。接入时必须同时增加真实测量源、单位与方向校验、硬目标判定及对应自测，再升级基线协议；当前协议拒绝把它们手工填成已测量值。这里不实现窗口、渲染、帧策略或控件功能。
