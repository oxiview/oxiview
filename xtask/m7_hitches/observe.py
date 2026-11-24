#!/usr/bin/env python3
"""Correlate Xcode 27 Animation Hitches with the viewer's exact Metal interval."""
import argparse
from bisect import bisect_left
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import re
import sqlite3
import subprocess
import xml.etree.ElementTree as ET

# With an XPath union xctrace only exports the first table's schema. Use the
# trace TOC to identify subsequent payloads, with checked Xcode 27 layouts.
COLUMNS = {
    "hitches": {0: ("start", "start-time"), 1: ("duration", "duration"),
                2: ("process", "process"), 3: ("is-system", "boolean"), 4: ("swap-id", "uint32"),
                6: ("display", "display-name")},
    "hitches-frame-lifetimes": {0: ("start", "start-time"), 1: ("duration", "duration"),
                                2: ("display", "display-name"), 3: ("swap-id", "uint32"),
                                4: ("surface-id", "uint32")},
    "displayed-surfaces-interval": {0: ("start", "start-time"), 3: ("display", "display-name"),
                                    5: ("surface-id", "uint64"),
                                    9: ("label", "narrative")},
    "time-info": {1: ("epoch", "mach-absolute-time"), 3: ("timebase", "mach-timebase-info")},
}
MATCH_NS = 200_000
PROCESS_PID_SUFFIX = re.compile(r" \(([0-9]+)\)$")
SURFACE_TAG = re.compile(r"\([^\r\n]+? \(([0-9]+)\):Frame [0-9,]+:Surface ")
BUILT_IN = re.compile(r"Built-In Display(?: [0-9]+)?")
TRACE_LOSS = re.compile(
    r"\b(?:data loss|(?:lost|dropped) (?:[\d,]+ )?(?:data|records?|events?|samples?)|"
    r"(?:records?|events?|samples?|(?:trace )?data) (?:were |was )?(?:lost|dropped)|"
    r"buffer overflow)\b", re.IGNORECASE,
)


def read_tables(paths, schema_names):
    tables = {}
    for path in paths:
        root = ET.parse(path).getroot()
        identities = {}
        for cell in root.iter():
            if (identity := cell.get("id")) is not None:
                if identity in identities:
                    raise ValueError(f"Duplicate Xcode export identity {identity}")
                identities[identity] = cell
        # Xcode can reference the PID separately from the process cell.
        for cell in root.iter("pid"):
            if (reference := cell.get("ref")) is not None:
                pid = identities.get(reference)
                cell.text = pid.text if pid is not None and pid.tag == "pid" else None
        for node in root.findall("node"):
            path_match = re.fullmatch(r"/+trace-toc(?:\[1\])?/run\[1\]/data(?:\[1\])?/table\[([1-9]\d*)\]",
                                      node.get("xpath", ""))
            if path_match is None:
                raise ValueError("Unexpected Xcode export table path")
            index = int(path_match.group(1)) - 1
            schema = schema_names[index]
            if schema not in COLUMNS:
                continue
            if schema in tables:
                raise ValueError(f"Ambiguous {schema} table")
            rows = []
            for row in node.findall("row"):
                values = {}
                for index, (name, kind) in COLUMNS[schema].items():
                    cell = row[index]
                    if "ref" in cell.attrib:
                        cell = identities[cell.get("ref")]
                    if cell.tag not in (kind, "sentinel"):
                        raise ValueError(f"Unexpected Xcode schema for {schema}.{name}: {cell.tag}")
                    values[name] = cell
                rows.append(values)
            tables[schema] = rows
    missing = COLUMNS.keys() - tables.keys()
    if missing:
        raise ValueError(f"Missing Xcode 27 Animation Hitches tables: {', '.join(sorted(missing))}")
    return tables


def number(row, key):
    return int(row[key].text)


def label(row, key):
    return row[key].get("fmt", "")


def parse_pid(text):
    value = (text or "").strip()
    return int(value) if value.isascii() and value.isdecimal() else None


def hitch_owner(cell):
    """Return the PID and whether the export claims a process owner."""
    if cell.tag == "sentinel":
        return None, False
    formatted = cell.get("fmt", "").strip()
    match = PROCESS_PID_SUFFIX.search(formatted)
    formatted_pid = parse_pid(match[1]) if match else None
    structured = [node.text for node in cell.findall("pid")]
    if "pid" in cell.attrib:
        structured.append(cell.get("pid"))
    if not structured:
        return formatted_pid, bool(formatted or (cell.text or "").strip())

    # A valid PID in the formatted label must agree with the structured owner.
    pids = {parse_pid(value) for value in structured}
    if len(pids) == 1:
        pid = pids.pop()
        if formatted_pid is None or pid == formatted_pid:
            return pid, True
    return None, True


def host_origin(clocks):
    if len(clocks) != 1:
        raise ValueError("Exactly one Instruments time base is required")
    row = clocks[0]
    numerator, denominator = map(int, label(row, "timebase").split("/"))
    epoch = number(row, "epoch")
    if numerator <= 0 or denominator <= 0 or epoch < 0:
        raise ValueError("Invalid Instruments time base")
    return epoch * numerator // denominator


def read_raw(path, output=None):
    # xctrace sends target stderr to this same file alongside JSON stdout.
    lines = path.read_text().splitlines()
    reports = [line for line in lines if line.startswith('{"rows":')]
    if len(reports) != 1:
        if not reports:
            error = next((line for line in reversed(lines) if line.startswith("log-viewer-oxi: ")), None)
            if error:
                raise ValueError(f"The native viewer did not finish the scroll: {error}")
        raise ValueError("Expected exactly one viewer JSON report")
    if output:
        output.write_text(reports[0] + "\n")
    return json.loads(reports[0])


def nearest(times, at):
    index = bisect_left(times, at)
    return min(times[max(0, index - 1):index + 1], key=lambda stamp: abs(stamp - at), default=None)


def matches_at(rows, at):
    """Find all timestamp candidates without accepting an ambiguous match."""
    index = bisect_left(rows, (at - MATCH_NS,))
    while index < len(rows) and rows[index][0] <= at + MATCH_NS:
        yield rows[index]
        index += 1


def merged_duration(spans):
    total = 0
    end = 0
    for lo, hi in sorted(spans):
        if hi > end:
            total += hi - max(lo, end)
            end = hi
    return total


def summarize(raw, tables, process, pid, trace_health="unavailable", trace_end_ns=None):
    """Attribute the sample by PID and retain the process label argument for replay callers."""
    if (raw["rows"], raw["warmup_seconds"], raw["sample_seconds"], raw["stride_points"]) != (100_000, 2, 10, 12):
        raise ValueError("Viewer did not run the fixed M7 workload")
    scale = raw["scale_factor"]
    if (not math.isfinite(scale) or scale <= 0 or
        abs(raw["pixel_width"] - 1120 * scale) > 0.5 or abs(raw["pixel_height"] - 720 * scale) > 0.5):
        raise ValueError("Viewer did not use the fixed M7 window geometry")
    if raw["missing_feedback"]:
        raise ValueError("Viewer did not produce complete Metal feedback")
    if not raw["valid"]:
        raise ValueError("Viewer marked the scroll workload invalid; inspect its frame records")
    if raw["expected_slots"] <= 0 or raw["interval_ns"] <= 0:
        raise ValueError("Invalid Metal display interval or slot count")
    origin = host_origin(tables["time-info"])
    start, end = (raw[key] - origin for key in ("start_ns", "end_ns"))
    half_interval = raw["interval_ns"] // 2
    presented = sorted(frame["presented_ns"] - origin for frame in raw["frames"]
                       if frame["presented_ns"])
    visible = [at for at in presented if start - half_interval <= at < end - half_interval]
    if start < 0 or len(visible) != raw["displayed_frames"] or not 9_990_000_000 <= end - start <= 10_010_000_000:
        raise ValueError("Invalid active scroll duration or inconsistent Metal frame count")
    if trace_end_ns is not None and end > trace_end_ns:
        raise ValueError("Animation Hitches recording ended before the active scroll interval")

    lifetimes_by_surface = defaultdict(list)
    lifetimes_by_swap = defaultdict(set)
    duplicate_lifetimes = set()
    malformed_lifetimes = defaultdict(int)
    for row in tables["hitches-frame-lifetimes"]:
        display = label(row, "display")
        try:
            beginning, duration = number(row, "start"), number(row, "duration")
            swap, surface = number(row, "swap-id"), number(row, "surface-id")
        except (TypeError, ValueError):
            if display:
                malformed_lifetimes[display] += 1
            continue
        finish = beginning + duration
        if not display or duration <= 0 or not 0 < swap < 2**32 or not 0 < surface < 2**32:
            if display and start - half_interval <= finish < end - half_interval:
                malformed_lifetimes[display] += 1
            continue
        key = (display, swap)
        if (finish, surface) in lifetimes_by_swap[key]:
            duplicate_lifetimes.add(key)
        else:
            lifetimes_by_swap[key].add((finish, surface))
            lifetimes_by_surface[surface].append((finish, display, swap))
    for rows in lifetimes_by_surface.values():
        rows.sort()

    joined = defaultdict(set)
    app_surfaces = defaultdict(set)
    foreign_surfaces = set()
    for row in tables["displayed-surfaces-interval"]:
        tag = SURFACE_TAG.search(label(row, "label"))
        if tag is None:
            continue
        display = label(row, "display")
        surface = number(row, "surface-id")
        at = number(row, "start")
        if int(tag[1]) != pid:
            if start - half_interval <= at < end - half_interval:
                foreign_surfaces.add((display, surface))
            continue
        closest = nearest(presented, at)
        if closest is None or abs(closest - at) > MATCH_NS:
            continue
        candidates = list(matches_at(lifetimes_by_surface[surface], at))
        if len(candidates) == 1:
            _, lifetime_display, swap = candidates[0]
            joined[display, lifetime_display].add((closest, swap))
            app_surfaces[display, lifetime_display].add(surface)

    pair = next(iter(joined)) if len(joined) == 1 else (None, None)
    surface_display, display = pair
    surfaces = {surface for surface in app_surfaces[pair]
                if (surface_display, surface) not in foreign_surfaces}
    # A surface ID is only meaningful within the display pair established by
    # the app tag and the same lifetime surface at its actual swap time.
    app_lifetimes = sorted(
        (at, swap) for surface in surfaces
        for at, name, swap in lifetimes_by_surface[surface]
        if name == display and len(lifetimes_by_swap[name, swap]) == 1
    )
    matched_swaps = set()
    for at in visible:
        candidates = list(matches_at(app_lifetimes, at))
        if len(candidates) == 1:
            matched_swaps.add(candidates[0][1])
    conflicts = malformed_lifetimes[display] + sum(
        (len(values) != 1 or (name, swap) in duplicate_lifetimes)
        and any(start - half_interval <= at < end - half_interval for at, _ in values)
        for (name, swap), values in lifetimes_by_swap.items() if name == display
    )
    joined_tags = joined.get(pair, set())
    tagged_matches = len({at for at, swap in joined_tags if swap in matched_swaps})
    app_swap_ids = {swap for _, swap in app_lifetimes}
    tag_times = [at for at, swap in joined_tags if swap in app_swap_ids]
    bracketed = (bool(tag_times) and bool(visible) and
                 min(tag_times) <= visible[0] and max(tag_times) >= visible[-1])
    hitch_spans = []
    intervals_by_swap = defaultdict(list)
    unmatched_rows = []
    seen = set()
    for row in tables["hitches"]:
        owner_pid, has_owner = hitch_owner(row["process"])
        if not has_owner and row["is-system"].text == "1":
            continue

        name = label(row, "display")
        try:
            swap = number(row, "swap-id")
        except (TypeError, ValueError):
            swap = None
        # The app's display and surface association can contradict a hitch owner.
        could_be_app = (owner_pid == pid or
                        (name in ("", display) and (owner_pid is None or swap in app_swap_ids)))
        if not could_be_app:
            continue
        try:
            beginning, duration = number(row, "start"), number(row, "duration")
        except (TypeError, ValueError):
            beginning = duration = None
        if None in (beginning, duration, swap):
            unmatched_rows.append({"reason": "missing Apple hitch interval or swap"})
            continue
        finish = beginning + duration
        if duration > 0 and (beginning >= end or finish <= start):
            continue
        identity = (owner_pid, name, swap, beginning, duration)
        if identity in seen:
            continue
        seen.add(identity)
        hitch = {"trace_start_ns": beginning, "duration_ns": duration, "swap_id": swap}
        if owner_pid != pid:
            reason = ("unresolved Apple hitch owner" if owner_pid is None else
                      "Apple hitch owner conflicts with the app swap")
            unmatched_rows.append(dict(hitch, display=name, owner_pid=owner_pid, reason=reason))
            continue
        lifetimes = lifetimes_by_swap.get((name, swap), ())
        closest_visible = nearest(presented, finish)
        if (duration <= 0 or name != display or len(lifetimes) != 1 or
            (trace_end_ns is not None and finish > trace_end_ns + MATCH_NS) or
            closest_visible is None or abs(closest_visible - finish) > MATCH_NS):
            unmatched_rows.append(dict(hitch, display=name, reason="unmatched display, swap or presentation"))
            continue
        life_end, surface = next(iter(lifetimes))
        if surface not in surfaces or abs(life_end - finish) > MATCH_NS:
            unmatched_rows.append(dict(hitch, display=name, reason="unmatched surface or lifetime end"))
            continue
        intervals_by_swap[swap].append(dict(hitch, surface_id=surface))

    observed = []
    for intervals in intervals_by_swap.values():
        if len(intervals) != 1:
            unmatched_rows.extend(dict(hitch, display=display, reason="conflicting hitch intervals for one swap")
                                  for hitch in intervals)
            continue
        hitch = intervals[0]
        beginning = hitch["trace_start_ns"]
        hitch_spans.append((max(start, beginning), min(end, beginning + hitch["duration_ns"])))
        observed.append(hitch)

    elapsed = end - start
    hitch_ns = merged_duration(hitch_spans)
    ratio = 1000 * hitch_ns / elapsed
    complete = (len(matched_swaps) == len(visible) and bool(visible) and not unmatched_rows and
                conflicts == 0 and bracketed and surface_display is not None and
                BUILT_IN.fullmatch(surface_display) is not None and trace_health == "no_loss_reported")
    missed_slot_time = 1000 * raw["dropped_frames"] / raw["expected_slots"]
    # An observed over-budget lower bound shows a failure even when coverage is incomplete.
    # Apple can miss long gaps before a drawable is submitted. The missed-slot
    # time can rule out a passing observation but cannot supply Apple hitch time.
    # Rust owns the final pass after validating the raw capture and reference clock.
    hard_pass = False if ratio > 1.2 else None
    return {
        "method": "Apple Instruments Animation Hitches",
        "app_pid": pid,
        "display": display,
        "surface_display": surface_display,
        "host_origin_ns": origin,
        "sample_start_trace_ns": start,
        "sample_end_trace_ns": end,
        "active_scroll_duration_ns": elapsed,
        "measured_metal_presentations": len(visible),
        "metal_reported_empty_slots": raw["dropped_frames"],
        "metal_missed_slot_ms_per_second": missed_slot_time,
        "apple_frame_lifetime_matches": len(matched_swaps),
        "app_tagged_surface_matches": tagged_matches,
        "app_surface_matches_bracket_sample": bracketed,
        "conflicting_apple_lifetimes": conflicts,
        "trace_health": trace_health,
        "unmatched_app_hitches": len(unmatched_rows),
        "unmatched_app_hitch_rows": unmatched_rows,
        "correlated_app_hitches": observed,
        "observed_hitch_time_ns": hitch_ns,
        "observed_hitch_ms_per_second": ratio,
        "complete_apple_frame_lifetime_coverage": complete,
        "threshold_ms_per_second": 1.2,
        "hard_check_pass": hard_pass,
    }


def trace_issues(trace, logs=()):
    """Read issue records Xcode stores with the run when that format is available."""
    store = trace / "Trace1.run" / "RunIssues.storedata"
    issues = []
    health = "unavailable"
    if store.is_file() and not store.with_name(store.name + "-wal").exists():
        try:
            with sqlite3.connect(f"{store.resolve().as_uri()}?mode=ro&immutable=1", uri=True) as db:
                rows = db.execute("SELECT ZRELATIVETIMESTAMP, ZCOUNT, ZMESSAGE FROM ZISSUE").fetchall()
            issues.extend({"trace_time_ns": at, "count": count, "message": message}
                          for at, count, message in rows)
            health = "no_loss_reported"
        except sqlite3.Error:
            pass
    for log in logs:
        if log.is_file():
            issues.extend({"source": log.name, "message": message}
                          for message in log.read_text(errors="replace").splitlines()
                          if TRACE_LOSS.search(message))
    if any(issue["message"] and TRACE_LOSS.search(issue["message"]) for issue in issues):
        health = "loss_reported"
    return health, issues


def export(trace, queries, log_path):
    with log_path.open("xb") as log:
        for query in queries:
            result = subprocess.run(["xcrun", "xctrace", "export", "--input", str(trace), *query],
                                    capture_output=True, timeout=300)
            log.write(result.stdout)
            log.write(result.stderr)
            result.check_returncode()


def check_recording_end(recorder_status, end_reason, target_status, expected_status):
    if end_reason != "Target app exited" or target_status != expected_status:
        raise ValueError("Trace did not end with the viewer's reported exit status")
    # Xcode 27 also returns 54 when it saves a completed recording of a
    # failing benchmark. Its TOC and the independently parsed viewer report
    # must both confirm that ordinary target exit.
    if recorder_status not in (None, 0, expected_status) and not (
        recorder_status == 54 and expected_status in (1, 2)
    ):
        raise ValueError(f"Animation Hitches recording failed ({recorder_status})")


def has_hitch_instruments(template, recording_log, selected_for_this_recording=False):
    if template == "Animation Hitches":
        return True
    # Xcode names a recording with explicitly selected instruments "Blank".
    # The launch receipt identifies the instruments; read_tables also checks
    # each required schema and cell type before any measurement is accepted.
    return template == "Blank" and (selected_for_this_recording or
            (recording_log.is_file() and
             "with the Blank template and Hitches, Display Instruments." in
             recording_log.read_text(errors="replace")))


def traced_executable(run, target, viewer):
    pid = parse_pid(target.get("pid"))
    if not pid:
        raise ValueError("Trace has no valid launched process PID")
    paths = {entry.get("path") for entry in [target, *run.findall("processes/process")]
             if parse_pid(entry.get("pid")) == pid and entry.get("path")}
    if len(paths) > 1:
        raise ValueError("Trace contains conflicting executable paths for the target PID")
    path = next(iter(paths), None)
    if viewer is not None:
        if path is None:
            raise ValueError(f"Trace has no executable path for the launched PID {pid}")
        recorded = Path(path)
        if not recorded.is_absolute() or recorded.resolve() != viewer.resolve():
            raise ValueError("Trace target disagrees with the release executable")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("export_prefix", type=Path)
    parser.add_argument("--toc", type=Path)
    parser.add_argument("--tables", nargs="+", type=Path)
    parser.add_argument("--viewer", type=Path, help="Record a fresh native scroll with this release executable")
    parser.add_argument("--raw-json", type=Path, help="Copy the original viewer JSON without xctrace stderr")
    args = parser.parse_args()
    recorder_status = None
    executable_hash = None
    logs = [args.trace.with_suffix(".record.log")]
    if args.viewer:
        if args.toc or args.tables:
            raise ValueError("Saved exports cannot be used with a fresh recording")
        log_path = logs[0]
        executable_hash = hashlib.sha256(args.viewer.read_bytes()).hexdigest()
        with log_path.open("x") as log:
            recorded = subprocess.run(
                ["xcrun", "xctrace", "record", "--instrument", "Hitches", "--instrument", "Display", "--no-prompt",
                 "--time-limit", "24s", "--output", str(args.trace), "--target-stdout", str(args.raw),
                 "--launch", "--", str(args.viewer), "--measure-scroll", "--json"],
                stdout=log, stderr=subprocess.STDOUT, timeout=420,
            )
        recorder_status = recorded.returncode
        if recorder_status not in (0, 1, 2, 54):
            raise ValueError(f"Animation Hitches recording failed ({recorded.returncode}); see {log_path}")
        if hashlib.sha256(args.viewer.read_bytes()).hexdigest() != executable_hash:
            raise ValueError("The release executable changed during the recording")
    if args.toc and args.tables:
        toc, paths = args.toc, args.tables
    elif args.toc or args.tables:
        raise ValueError("Supply both --toc and --tables to use saved exports")
    else:
        toc = Path(str(args.export_prefix) + "-toc.xml")
        path = Path(str(args.export_prefix) + "-tables.xml")
        criteria = " or ".join(f'@schema="{schema}"' for schema in COLUMNS)
        logs.append(Path(str(args.export_prefix) + "-export.log"))
        export(args.trace, [
            ["--toc", "--output", str(toc)],
            ["--xpath", f'/trace-toc/run[@number="1"]/data/table[{criteria}]', "--output", str(path)],
        ], logs[-1])
        paths = [path]

    run = ET.parse(toc).find("./run[@number='1']")
    if run is None:
        raise ValueError("Missing Animation Hitches recording run")
    processes = run.findall("info/target/process")
    if (len(processes) != 1 or processes[0].get("type") != "launched" or
        not has_hitch_instruments(run.findtext("info/summary/template-name"), logs[0],
                                  selected_for_this_recording=args.viewer is not None)):
        raise ValueError("Trace was not launched for the viewer using Animation Hitches")
    process = processes[0]
    if run.findtext("info/summary/end-reason") != "Target app exited":
        raise ValueError(f"Recording ended before normal viewer exit: "
                         f"{run.findtext('info/summary/end-reason')}, "
                         f"target {process.get('termination-reason', 'unknown')}")
    if process.get("arguments") != "--measure-scroll --json":
        raise ValueError("Trace did not run the M7 scroll command")
    target_path = traced_executable(run, process, args.viewer)
    names = [table.get("schema") for table in run.findall("data/table")]
    raw = read_raw(args.raw, args.raw_json)
    expected_status = int(not raw["pass"]) if raw["valid"] else 2
    check_recording_end(recorder_status, run.findtext("info/summary/end-reason"),
                        int(process.get("return-exit-status", "-1")), expected_status)
    trace_seconds = float(run.findtext("info/summary/duration", default="nan"))
    if not math.isfinite(trace_seconds) or trace_seconds <= 0:
        raise ValueError("Invalid Animation Hitches recording duration")
    health, issues = trace_issues(args.trace, logs)
    summary = summarize(raw, read_tables(paths, names), process.get("name"), int(process.get("pid")),
                        health, round(trace_seconds * 1_000_000_000))
    summary["trace_issues"] = issues
    summary["trace_executable"] = target_path
    if args.viewer:
        summary["recorder_exit_status"] = recorder_status
        summary["executable_sha256"] = executable_hash
    summary["target_exit_status"] = int(process.get("return-exit-status"))
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        diagnostic = error.stderr.decode(errors="replace").strip() if error.stderr else "see recording log"
        raise SystemExit(f"hitch-observer: {error}: {diagnostic}")
    except (ValueError, TypeError, OSError, KeyError, IndexError, ET.ParseError, subprocess.TimeoutExpired) as error:
        raise SystemExit(f"hitch-observer: {error}")
