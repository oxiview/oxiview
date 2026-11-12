//! 本机性能基线与 Divan 中位数判定。

use crate::{check::Check, report::Status};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::SystemTime};

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Lower,
    Higher,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Metric {
    value: Option<f64>,
    unit: String,
    direction: Direction,
    #[serde(default = "default_threshold")]
    regression_percent: f64,
    target: Option<f64>,
}
fn default_threshold() -> f64 {
    10.0
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    model: String,
    cpu: String,
    memory_bytes: u64,
    os: String,
    rustc: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    schema_version: u32,
    machine_id: String,
    machine: Machine,
    recorded_at_unix_seconds: u64,
    metrics: BTreeMap<String, Metric>,
}

fn command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{program} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(format!("{program} 返回空机器信息"));
    }
    Ok(text)
}

// mini 模式排除序列号与 UUID；只提取与 sysctl 等价的硬件字段。
fn macos_hardware_fallback() -> Result<(String, String, String)> {
    let text = command(
        "system_profiler",
        &["SPHardwareDataType", "-detailLevel", "mini", "-json"],
    )?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let hardware = &data["SPHardwareDataType"][0];
    let field = |name| {
        hardware[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("缺少硬件字段：{name}"))
    };
    let model = field("machine_model")?;
    let cpu = field("chip_type").or_else(|_| field("cpu_type"))?;
    let memory = field("physical_memory")?;
    let words: Vec<_> = memory.split_whitespace().collect();
    if words.len() != 2 {
        return Err("无法解析物理内存".into());
    }
    let scale = match words[1] {
        "GB" => 1_u64 << 30,
        "MB" => 1_u64 << 20,
        "TB" => 1_u64 << 40,
        _ => return Err("未知内存单位".into()),
    };
    let bytes = words[0]
        .parse::<u64>()
        .map_err(|e| e.to_string())?
        .checked_mul(scale)
        .ok_or("内存数值溢出")?;
    Ok((model, cpu, bytes.to_string()))
}

fn machine() -> Result<Machine> {
    let (model, cpu, memory, os) = if cfg!(target_os = "macos") {
        let hardware = (|| {
            Ok::<_, String>((
                command("sysctl", &["-n", "hw.model"])?,
                command("sysctl", &["-n", "machdep.cpu.brand_string"])?,
                command("sysctl", &["-n", "hw.memsize"])?,
            ))
        })()
        .or_else(|_| macos_hardware_fallback())?;
        (
            hardware.0,
            hardware.1,
            hardware.2,
            command("sw_vers", &["-productVersion"])?,
        )
    } else if cfg!(target_os = "linux") {
        let cpu = fs::read_to_string("/proc/cpuinfo").map_err(|e| e.to_string())?;
        let memory = fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
        let kb: u64 = memory
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.split_whitespace().next())
            .ok_or("缺少内存信息")?
            .parse::<u64>()
            .map_err(|e| e.to_string())?;
        (
            fs::read_to_string("/sys/class/dmi/id/product_name")
                .map_err(|e| e.to_string())?
                .trim()
                .into(),
            cpu.lines()
                .find_map(|l| {
                    l.strip_prefix("model name")
                        .and_then(|v| v.split_once(':'))
                        .map(|(_, v)| v.trim().to_owned())
                })
                .ok_or("缺少 CPU 型号")?,
            (kb * 1024).to_string(),
            command("uname", &["-sr"])?,
        )
    } else if cfg!(target_os = "windows") {
        let ps = |s| {
            command(
                "powershell",
                &["-NoProfile", "-NonInteractive", "-Command", s],
            )
        };
        (
            ps("(Get-CimInstance Win32_ComputerSystem).Model")?,
            ps("(Get-CimInstance Win32_Processor | Select-Object -First 1).Name")?,
            ps("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory")?,
            ps("(Get-CimInstance Win32_OperatingSystem).Version")?,
        )
    } else {
        return Err("只支持 macOS / Windows / Linux 原生平台".into());
    };
    Ok(Machine {
        model,
        cpu,
        memory_bytes: memory.parse::<u64>().map_err(|e| e.to_string())?,
        os: format!("{} {os}", std::env::consts::OS),
        rustc: command("rustc", &["--version"])?,
    })
}

fn machine_id(machine: &Machine) -> String {
    // 固定 FNV-1a，避免标准库哈希算法或随机种子变化；不采集序列号。
    let hardware = format!(
        "{}\n{}\n{}\n{}",
        std::env::consts::ARCH,
        machine.model,
        machine.cpu,
        machine.memory_bytes
    );
    let hash = hardware.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{}-{hash:016x}", std::env::consts::OS)
}

fn duration_ns(text: &str) -> Result<f64> {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.len() != 2 {
        return Err(format!("无效时间：{text}"));
    }
    let scale = match words[1] {
        "ps" => 0.001,
        "ns" => 1.0,
        "µs" | "μs" | "us" => 1_000.0,
        "ms" => 1_000_000.0,
        "s" => 1_000_000_000.0,
        _ => return Err(format!("未知时间单位：{text}")),
    };
    let value = words[0].parse::<f64>().map_err(|e| e.to_string())? * scale;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("无效测量：{text}"));
    }
    Ok(value)
}

fn parse_divan(text: &str) -> Result<BTreeMap<String, f64>> {
    let mut values = BTreeMap::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut header = false;
    for line in text.lines() {
        let content = line.trim_start_matches(|c: char| c.is_whitespace() || "├╰─│".contains(c));
        let prefix = line.chars().count() - content.chars().count();
        let columns: Vec<_> = content.split('│').collect();
        if line.contains("fastest") && line.contains("median") {
            if columns.len() != 6 || columns[2].trim() != "median" {
                return Err("Divan 表头发生变化".into());
            }
            parents.clear();
            let name = columns[0]
                .split_whitespace()
                .next()
                .ok_or("缺少基准 target")?;
            parents.push((0, name.into()));
            header = true;
            continue;
        }
        if !header || !line.contains('─') {
            continue;
        }
        if columns.len() != 6 {
            return Err(format!("Divan 行格式发生变化：{line}"));
        }
        let name = columns[0].split_whitespace().next().ok_or("空基准名称")?;
        while parents.last().is_some_and(|(depth, _)| *depth >= prefix) {
            parents.pop();
        }
        let mut path: Vec<_> = parents.iter().map(|(_, n)| n.as_str()).collect();
        path.push(name);
        if columns[2].trim().is_empty() {
            if columns
                .iter()
                .skip(1)
                .any(|column| !column.trim().is_empty())
            {
                return Err("Divan 统计列不完整".into());
            }
            parents.push((prefix, name.into()));
            continue;
        }
        let first: Vec<_> = columns[0].split_whitespace().collect();
        if first.len() != 3 {
            return Err("基准名称必须不含空白且具有 fastest 统计量".into());
        }
        duration_ns(&first[1..].join(" "))?;
        // 每个叶子都必须有完整统计量，禁止静默跳过解析失败的指标。
        duration_ns(columns[1])?;
        let value = duration_ns(columns[2])?;
        duration_ns(columns[3])?;
        for count in &columns[4..] {
            if count.trim().parse::<u64>().map_err(|e| e.to_string())? == 0 {
                return Err("空采样".into());
            }
        }
        let key = path.join("/");
        if values.insert(key.clone(), value).is_some() {
            return Err(format!("重复指标：{key}"));
        }
    }
    if values.is_empty() {
        return Err("没有解析到 Divan 指标".into());
    }
    Ok(values)
}

fn pending() -> BTreeMap<String, Metric> {
    [
        ("input_to_pixel_ms", "ms", 8.0),
        ("idle_cpu_percent", "%", 0.0),
        ("list_100k_scroll_dropped_frames", "frames", 0.0),
    ]
    .into_iter()
    .map(|(name, unit, target)| {
        (
            name.into(),
            Metric {
                value: None,
                unit: unit.into(),
                direction: Direction::Lower,
                regression_percent: default_threshold(),
                target: Some(target),
            },
        )
    })
    .collect()
}

fn validate(baseline: &Baseline) -> Result<()> {
    if baseline.schema_version != 1 || baseline.machine_id != machine_id(&baseline.machine) {
        return Err("基线版本或机器标识无效".into());
    }
    for (name, metric) in &baseline.metrics {
        if !metric.regression_percent.is_finite()
            || !(0.0..=100.0).contains(&metric.regression_percent)
            || metric.value.is_some_and(|v| !v.is_finite() || v < 0.0)
        {
            return Err(format!("基线指标无效：{name}"));
        }
        if let Some(reserved) = pending().get(name) {
            if metric.value.is_some()
                || metric.target != reserved.target
                || metric.unit != reserved.unit
                || metric.direction != reserved.direction
            {
                return Err(format!("硬指标在接入真实测量前必须 pending：{name}"));
            }
        } else if metric.value.is_none()
            || metric.unit != "ns"
            || metric.direction != Direction::Lower
        {
            return Err(format!("微基准指标元数据不兼容：{name}"));
        }
    }
    if pending()
        .keys()
        .any(|key| !baseline.metrics.contains_key(key))
    {
        return Err("基线缺少硬指标预留项".into());
    }
    Ok(())
}

fn regressed(current: f64, previous: f64, direction: &Direction, percent: f64) -> bool {
    let allowance = previous.abs() * percent / 100.0;
    match direction {
        Direction::Lower => current - previous > allowance,
        Direction::Higher => previous - current > allowance,
    }
}

pub(crate) struct BenchCheck {
    pub record: bool,
}

impl BenchCheck {
    fn process(&self, root: &Path, log: &mut String) -> Result<Status> {
        for artifact in [
            "bench-results.json",
            "bench-1.log",
            "bench-2.log",
            "bench-3.log",
        ] {
            match fs::remove_file(root.join("target/xtask").join(artifact)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let machine = machine()?;
        let id = machine_id(&machine);
        log.push_str(&format!("当前机器：{id}，{machine:?}\n"));
        let directory = root.join("perf/baselines");
        let path = directory.join(format!("{id}.json"));
        let previous: Option<Baseline> = match fs::read(&path) {
            Ok(bytes) => {
                let baseline = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                validate(&baseline)?;
                if baseline.machine_id != id {
                    return Err("基线文件名与当前机器不匹配".into());
                }
                Some(baseline)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if directory.exists() {
            let mut paths = fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            paths.sort();
            for other in paths {
                if other != path && other.extension().is_some_and(|e| e == "json") {
                    log.push_str(&format!(
                        "其他机器（仅显示，不判定）{}：\n{}\n",
                        other.display(),
                        fs::read_to_string(&other).map_err(|e| e.to_string())?
                    ));
                }
            }
        }
        let mut rounds = Vec::new();
        for round in 1..=3 {
            let mut command =
                Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
            command
                .args(["bench", "--workspace", "--benches", "--locked"])
                .current_dir(root)
                .env("CARGO_TERM_COLOR", "never");
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("DIVAN_") {
                    command.env_remove(key);
                }
            }
            let output = command.output().map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let raw = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
            fs::write(root.join(format!("target/xtask/bench-{round}.log")), raw)
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                log.push_str(&format!(
                    "基准进程失败：{}，见 bench-{round}.log\n",
                    output.status
                ));
                return Ok(if output.status.code().is_some() {
                    Status::Fail
                } else {
                    Status::Error
                });
            }
            rounds.push(parse_divan(&stdout)?);
        }
        if rounds
            .iter()
            .skip(1)
            .any(|r| !r.keys().eq(rounds[0].keys()))
        {
            return Err("三轮指标集合不一致".into());
        }
        let mut metrics = pending();
        for name in rounds[0].keys() {
            let mut samples: Vec<_> = rounds.iter().map(|round| round[name]).collect();
            samples.sort_by(f64::total_cmp);
            let threshold = previous
                .as_ref()
                .and_then(|b| b.metrics.get(name))
                .map_or(default_threshold(), |m| m.regression_percent);
            metrics.insert(
                name.clone(),
                Metric {
                    value: Some(samples[1]),
                    unit: "ns".into(),
                    direction: Direction::Lower,
                    regression_percent: threshold,
                    target: None,
                },
            );
        }
        let mut status = Status::Pass;
        let mut results = Vec::new();
        for (name, metric) in &metrics {
            let old = previous
                .as_ref()
                .and_then(|b| b.metrics.get(name))
                .and_then(|m| m.value);
            let state = match metric.value {
                None => "pending",
                Some(_) if self.record => "recorded",
                Some(value) => match old {
                    None => "missing_baseline",
                    Some(old)
                        if regressed(value, old, &metric.direction, metric.regression_percent) =>
                    {
                        "fail"
                    }
                    Some(_) => "pass",
                },
            };
            if ["fail", "missing_baseline"].contains(&state) {
                status = Status::Fail;
            }
            log.push_str(&format!(
                "{name}：{state}，当前 {:?} {}，基线 {old:?}，目标 {:?}，允许回归 {}%\n",
                metric.value, metric.unit, metric.target, metric.regression_percent
            ));
            results.push(serde_json::json!({"name": name, "status": state, "current": metric, "baseline_value": old}));
        }
        if let Some(old) = &previous {
            for (name, metric) in &old.metrics {
                if !metrics.contains_key(name) {
                    status = Status::Fail;
                    log.push_str(&format!("缺失已记录指标：{name}；禁止静默删除\n"));
                    results.push(serde_json::json!({"name":name,"status":"missing_measurement","baseline_value":metric.value}));
                }
            }
        }
        if previous.is_none() && !self.record {
            log.push_str("缺少本机基线；请人工确认后运行 cargo xtask bench --record。\n");
        }
        fs::write(
            root.join("target/xtask/bench-results.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"machine_id":id,"metrics":results}))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if self.record && status == Status::Pass {
            let baseline = Baseline {
                schema_version: 1,
                machine_id: id,
                machine,
                recorded_at_unix_seconds: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
                metrics,
            };
            fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            fs::write(
                &path,
                serde_json::to_vec_pretty(&baseline).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            log.push_str(&format!(
                "已创建或覆盖仓库基线文件：{}；请审查并提交。\n",
                path.display()
            ));
        }
        Ok(status)
    }
}
impl Check for BenchCheck {
    fn name(&self) -> &str {
        "bench"
    }
    fn run(&self, root: &Path) -> (Status, String) {
        let mut log = String::new();
        match self.process(root, &mut log) {
            Ok(status) => (status, log),
            Err(error) => (Status::Error, format!("{log}{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thresholds_and_directions() {
        assert!(!regressed(110.0, 100.0, &Direction::Lower, 10.0));
        assert!(regressed(110.01, 100.0, &Direction::Lower, 10.0));
        assert!(!regressed(90.0, 100.0, &Direction::Higher, 10.0));
        assert!(regressed(89.0, 100.0, &Direction::Higher, 10.0));
        assert!(regressed(1.0, 0.0, &Direction::Lower, 10.0));
    }
    #[test]
    fn parser_requires_complete_statistics() {
        let text = "compare fastest │ slowest │ median │ mean │ samples │ iters\n╰─ image_compare │ │ │ │ │\n   ├─ 256 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 1000\n   ╰─ 1024 1 ms │ 3 ms │ 2 ms │ 2 ms │ 100 │ 1000";
        let result = parse_divan(text).expect("合法输出");
        assert_eq!(result["compare/image_compare/256"], 2000.0);
        assert_eq!(result["compare/image_compare/1024"], 2_000_000.0);
        for invalid in [
            text.replace("2 µs", "NaN µs"),
            text.replace("2 ms", "2 ticks"),
            text.replace("median", "p50"),
            String::new(),
        ] {
            assert!(parse_divan(&invalid).is_err());
        }
    }
    #[test]
    fn parser_handles_nested_branches_and_duplicate_metrics() {
        let text = "bench fastest │ slowest │ median │ mean │ samples │ iters\n├─ first │ │ │ │ │\n│  ╰─ 1 1 ns │ 3 ns │ 2 ns │ 2 ns │ 10 │ 10\n╰─ second 1 ns │ 4 ns │ 3 ns │ 3 ns │ 10 │ 10";
        let values = parse_divan(text).expect("嵌套输出");
        assert_eq!(values["bench/first/1"], 2.0);
        assert_eq!(values["bench/second"], 3.0);
        assert!(parse_divan(&format!("{text}\n{text}")).is_err());
    }

    #[test]
    fn baseline_rejects_invalid_thresholds_and_fake_hard_measurements() {
        let machine = Machine {
            model: "model".into(),
            cpu: "cpu".into(),
            memory_bytes: 100,
            os: "os".into(),
            rustc: "rustc".into(),
        };
        let mut baseline = Baseline {
            schema_version: 1,
            machine_id: machine_id(&machine),
            machine,
            recorded_at_unix_seconds: 1,
            metrics: pending(),
        };
        assert!(validate(&baseline).is_ok());
        baseline
            .metrics
            .get_mut("idle_cpu_percent")
            .expect("硬指标")
            .value = Some(0.0);
        assert!(validate(&baseline).is_err());
        baseline.metrics = pending();
        baseline
            .metrics
            .get_mut("idle_cpu_percent")
            .expect("硬指标")
            .regression_percent = -1.0;
        assert!(validate(&baseline).is_err());
        let metric: Metric =
            serde_json::from_str(r#"{"value":1.0,"unit":"ns","direction":"lower","target":null}"#)
                .expect("默认阈值");
        assert_eq!(metric.regression_percent, 10.0);
    }

    #[test]
    fn identity_excludes_software_and_pending_has_no_measurement() {
        let mut m = Machine {
            model: "model".into(),
            cpu: "cpu".into(),
            memory_bytes: 100,
            os: "os".into(),
            rustc: "rustc".into(),
        };
        let id = machine_id(&m);
        m.os = "new".into();
        m.rustc = "new".into();
        assert_eq!(id, machine_id(&m));
        m.memory_bytes += 1;
        assert_ne!(id, machine_id(&m));
        assert_eq!(pending().len(), 3);
        assert!(pending().values().all(|m| m.value.is_none()));
    }
}
