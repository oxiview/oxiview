use super::{Animation, Native, Run, ScrollReport};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(super) enum Lang {
    En,
    Zh,
}

impl Lang {
    fn text<'a>(self, en: &'a str, zh: &'a str) -> &'a str {
        match self {
            Self::En => en,
            Self::Zh => zh,
        }
    }
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn label(run: &Run, lang: Lang) -> String {
    format!(
        "{} `{}`{}",
        cell(lang.text(&run.meta.label[0], &run.meta.label[1])),
        &run.meta.head[..7],
        if run.meta.dirty { "\\*" } else { "" }
    )
}

fn table_row(page: &mut String, cells: impl IntoIterator<Item = String>) {
    page.push_str("| ");
    page.push_str(&cells.into_iter().collect::<Vec<_>>().join(" | "));
    page.push_str(" |\n");
}

fn native(page: &mut String, runs: &[&Run], lang: Lang) {
    let values: Vec<_> = runs
        .iter()
        .filter_map(|r| r.native.as_ref().map(|n| (*r, n)))
        .collect();
    if values.is_empty() {
        return;
    }
    table_row(
        page,
        std::iter::once(lang.text("Metric", "指标").into())
            .chain(values.iter().map(|(r, _)| label(r, lang))),
    );
    table_row(
        page,
        std::iter::once("---".into()).chain(std::iter::repeat_n("---:".into(), values.len())),
    );
    let mut row = |name: &str, value: &dyn Fn(&Run, &Native) -> String| {
        table_row(
            page,
            std::iter::once(name.to_owned()).chain(values.iter().map(|(r, n)| value(r, n))),
        );
    };
    row(
        lang.text("Windowed input to submission, P95", "窗口输入至提交，P95"),
        &|r, n| format!("{:.*} ms", r.meta.latency_decimals.unwrap_or(4), n.submit),
    );
    row(
        lang.text("Windowed input to display, P95", "窗口输入至实际呈现，P95"),
        &|r, n| format!("{:.*} ms", r.meta.latency_decimals.unwrap_or(4), n.display),
    );
    row(
        lang.text(
            "Idle process CPU (observation length)",
            "空闲进程 CPU（观测时长）",
        ),
        &|_, n| format!("{:.5}% ({:.0} s)", n.idle_cpu, n.idle_seconds),
    );
    row(
        lang.text(
            "Idle frames submitted / presented / display-link events",
            "空闲提交帧 / 呈现帧 / 显示链路事件",
        ),
        &|_, n| format!("{} / {} / {}", n.counts[0], n.counts[1], n.counts[2]),
    );
    for (i, count) in ["1", "128", "1,000"].iter().enumerate() {
        let metric = match lang {
            Lang::En => format!(
                "Offscreen GPU, {count} sparse rounded rectangle{}",
                if i == 0 { "" } else { "s" }
            ),
            Lang::Zh => format!("离屏 GPU，{count} 个稀疏圆角矩形"),
        };
        row(&metric, &|_, n| format!("{:.3} ms", n.gpu_ms[i]));
    }
    page.push('\n');
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn fps(samples: &[&Animation]) -> String {
    let mut text = format!("{:.2}", median(samples.iter().map(|a| a.displayed_fps)));
    if text.ends_with('0') {
        text.pop();
    }
    text
}

fn animation(page: &mut String, runs: &[&Run], lang: Lang) {
    // Compare resolutions within a mode and durations within a workload.
    let mut workloads = BTreeMap::new();
    for (i, run) in runs.iter().enumerate() {
        for sample in &run.animation {
            let c = &sample.conditions;
            let workload = (
                c.objects,
                c.scale.to_bits(),
                sample.warmup_seconds.to_bits(),
            );
            let row = (c.fullscreen, c.pixel_width, c.pixel_height, i);
            workloads
                .entry(workload)
                .or_insert_with(BTreeMap::new)
                .entry(row)
                .or_insert_with(BTreeMap::new)
                .entry(sample.sample_seconds.to_bits())
                .or_insert_with(Vec::new)
                .push(sample);
        }
    }
    for ((objects, scale, warmup), rows) in workloads {
        let scale = f64::from_bits(scale);
        let warmup = f64::from_bits(warmup);
        page.push_str(&match lang {
            Lang::En=>format!("The release build rendered {objects} moving objects and the metrics panel at {scale}× scale after a {warmup} s warm-up. Displayed FPS counts Metal-confirmed presentations. Repeated runs are summarized by their median.\n\n"),
            Lang::Zh=>format!("Release 构建在 {scale}× 缩放下绘制 {objects} 个运动对象和统计面板，预热 {warmup} 秒。显示 FPS 只计入 Metal 确认呈现的帧，多轮结果取中位数。\n\n"),
        });
        page.push_str(lang.text(
            "| Run | Mode and physical pixels | Samples | Displayed FPS | Display gap P95 | Longer observation |\n",
            "| 测试记录 | 模式与物理像素 | 采样 | 显示 FPS | 帧间隔 P95 | 较长观测 |\n"));
        page.push_str("| --- | --- | ---: | ---: | ---: | ---: |\n");
        for ((full, width, height, i), durations) in rows {
            let mut groups = durations.into_iter();
            let Some((duration, samples)) = groups.next() else {
                continue;
            };
            let longer = groups
                .map(|(s, a)| format!("{} FPS ({} × {} s)", fps(&a), a.len(), f64::from_bits(s)))
                .collect::<Vec<_>>();
            let mode = if full {
                lang.text("Fullscreen FIFO", "全屏 FIFO")
            } else {
                lang.text("Windowed", "窗口")
            };
            table_row(
                page,
                [
                    label(runs[i], lang),
                    format!("{mode} · {width} × {height}"),
                    format!("{} × {} s", samples.len(), f64::from_bits(duration)),
                    fps(&samples),
                    format!(
                        "{:.2} ms",
                        median(samples.iter().map(|a| a.display_gap_p95_ms))
                    ),
                    if longer.is_empty() {
                        "—".into()
                    } else {
                        longer.join("<br>")
                    },
                ],
            );
        }
        page.push('\n');
    }
    for r in runs {
        let probes = &r.meta.pointer_probes;
        if !probes.is_empty() {
            let mut values = probes
                .iter()
                .map(|p| format!("{:.1} ms", p.display_p95_ms))
                .collect::<Vec<_>>();
            let last = values.pop().unwrap_or_default();
            let p95 = if values.is_empty() {
                last
            } else {
                format!("{}{}{last}", values.join(", "), lang.text(" and ", " 和 "))
            };
            let visible: u64 = probes.iter().map(|p| p.visible).sum();
            let inputs: u64 = probes.iter().map(|p| p.inputs).sum();
            page.push_str(&match lang {
                Lang::En=>format!("Fullscreen pointer probes for {} measured capture-to-display P95 at {p95}. Metal presented {visible} of {inputs} input markers.",label(r,lang)),
                Lang::Zh=>format!("{} 的独立全屏指针探针测得输入捕获至实际呈现 P95 为 {p95}，上屏标记为 {visible}/{inputs} 个。",label(r,lang)),
            });
        }
        if let Some(ms) = r.meta.continuous_submit_ms {
            if matches!(lang, Lang::En) && !probes.is_empty() {
                page.push(' ');
            }
            page.push_str(&match lang {
                Lang::En => format!(
                    "A separate sustained-input run measured input-to-submit P95 at {ms:.2} ms."
                ),
                Lang::Zh => format!("另一项持续输入测试测得输入至提交 P95 为 {ms:.2} ms。"),
            });
        }
        if !probes.is_empty() || r.meta.continuous_submit_ms.is_some() {
            page.push_str("\n\n");
        }
    }
}

fn scroll(page: &mut String, report: &ScrollReport, lang: Lang) {
    table_row(
        page,
        [
            lang.text("100,000-row native scroll", "十万行原生滚动")
                .into(),
            lang.text("Displayed / expected slots", "上屏 / 预期时隙")
                .into(),
            lang.text("Measurement", "测量结果").into(),
        ],
    );
    page.push_str("| --- | ---: | ---: |\n");
    let mut cadence_round = 0;
    let mut trace_round = 0;
    for sample in &report.samples {
        let elapsed = sample.measured_duration_ns as f64;
        let (capture, value) = if let Some(hitch) = &sample.apple {
            trace_round += 1;
            let rate = 1000.0 * hitch.observed_hitch_time_ns as f64 / elapsed;
            let qualifier = if hitch.complete_apple_frame_lifetime_coverage {
                ""
            } else {
                lang.text(" (lower bound)", "（下限）")
            };
            (
                format!("{} {trace_round}", lang.text("Apple trace", "Apple 跟踪")),
                format!("{rate:.2} ms/s{qualifier}"),
            )
        } else {
            cadence_round += 1;
            let fps = 1e9 * sample.occupied_slots as f64 / elapsed;
            (
                format!("{} {cadence_round}", lang.text("Untraced", "无跟踪")),
                format!("{fps:.1} FPS"),
            )
        };
        table_row(
            page,
            [
                capture,
                format!("{} / {}", sample.occupied_slots, sample.expected_slots),
                value,
            ],
        );
    }
    page.push('\n');
}

pub(super) fn summary(runs: &[Run], lang: Lang) -> String {
    let mut page = String::new();
    let mut systems = BTreeMap::<_, Vec<_>>::new();
    for run in runs {
        systems.entry(&run.meta.machine).or_default().push(run);
    }
    for (host, group) in systems {
        page.push_str(&format!(
            "**{}** · {}\n\n",
            lang.text("Test system", "测试环境"),
            cell(host)
        ));
        for run in &group {
            if let Some(report) = &run.scroll {
                page.push_str(&format!("**{}**\n\n", label(run, lang)));
                scroll(&mut page, report, lang);
            }
        }
        if group.iter().any(|r| r.native.is_some()) {
            page.push_str(lang.text(
                "### Window latency, idle CPU and offscreen rendering\n\n",
                "### 窗口延迟、空闲 CPU 与离屏渲染\n\n",
            ));
            page.push_str(lang.text(
                "The macOS probe starts timing just before each synthetic AppKit pointer event is posted. It measures 120 events per round and reports the median of three round P95s. Submission ends when the presenter returns. Display latency ends at Metal’s actual presentation timestamp. The 1280 × 840 offscreen test includes scene encoding, GPU completion and RGBA readback.\n\n",
                "macOS 探针从投递每个合成 AppKit 指针事件前开始计时。每轮测量 120 个事件，延迟取三轮 P95 的中位数。提交延迟计至呈现器返回，显示延迟计至 Metal 记录的实际上屏时刻。1280 × 840 离屏测试计入场景编码、GPU 完成和 RGBA 回读。\n\n"));
            native(&mut page, &group, lang);
        }
        if group.iter().any(|r| !r.animation.is_empty()) {
            page.push_str(lang.text(
                "### Continuous animation and input latency\n\n",
                "### 连续动画与输入延迟\n\n",
            ));
            animation(&mut page, &group, lang);
        }
    }
    if runs.iter().any(|r| r.meta.dirty) {
        page.push_str(lang.text(
            "An asterisk marks a measurement taken with local changes on top of the displayed Git HEAD. The archive identifies the commits that subsequently landed the measured changes.\n",
            "星号表示测量时工作树含本地改动，哈希为当时的 Git HEAD。对应改动后来落入的提交见历史归档。\n"));
    }
    page
}

pub(super) fn history(runs: &[Run]) -> String {
    let mut page = String::from(
        "# Performance history\n\nAll archived measurement campaigns appear below, newest first. Each heading identifies the Git HEAD at measurement time. An asterisk indicates local changes. Each run links to its original benchmark or animation samples. [Measurement protocol](../conventions/testing-performance.md)\n\n",
    );
    for run in runs.iter().rev() {
        page.push_str(&format!(
            "## {}\n\n{}\n\nTest HEAD `{}`.",
            label(run, Lang::En),
            cell(&run.meta.machine),
            run.meta.head
        ));
        if let Some(implementation) = &run.meta.implementation {
            page.push_str(&format!(
                " Related implementation subsequently committed as `{}`.",
                &implementation[..7]
            ));
        }
        page.push_str("\n\n");
        if let Some(report) = &run.scroll {
            scroll(&mut page, report, Lang::En);
        }
        native(&mut page, &[run], Lang::En);
        animation(&mut page, &[run], Lang::En);
        let dir = run.id();
        let links = run
            .files
            .keys()
            .map(|name| format!("[{name}](runs/{dir}/{name})"))
            .collect::<Vec<_>>();
        page.push_str(&links.join(" · "));
        page.push_str("\n\n");
    }
    format!("{}\n", page.trim_end())
}
