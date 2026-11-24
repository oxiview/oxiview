"""Exercise the observer parser without recording a trace or opening a window."""
import io
from pathlib import Path
import sqlite3
import unittest
from unittest.mock import Mock, patch
import xml.etree.ElementTree as ET

from observe import (check_recording_end, has_hitch_instruments, read_tables, summarize,
                     trace_issues, traced_executable)


NS = 1_000_000_000
PERIOD = NS // 100
HOST = 1_000 * NS
START = 3 * NS
END = START + 10 * NS


def cell(tag, value, fmt=None):
    node = ET.Element(tag, fmt=str(value) if fmt is None else fmt)
    node.text = str(value)
    return node


def evidence():
    raw = {
        "rows": 100_000, "warmup_seconds": 2, "sample_seconds": 10, "stride_points": 12,
        "pixel_width": 2240, "pixel_height": 1440, "scale_factor": 2,
        "missing_feedback": 0, "valid": True,
        "start_ns": HOST + START, "end_ns": HOST + END, "interval_ns": PERIOD,
        "displayed_frames": 1000, "dropped_frames": 0, "expected_slots": 1000,
        "frames": [{"presented_ns": HOST + START + slot * PERIOD} for slot in range(1002)],
    }
    lifetimes = []
    surfaces = []
    for slot in range(1002):
        at = START + slot * PERIOD
        lifetimes.append({"start": cell("start-time", at - 1), "duration": cell("duration", 1),
                          "display": cell("display-name", "Display 1"), "swap-id": cell("uint32", slot + 100),
                          "surface-id": cell("uint32", 10)})
        surfaces.append({"start": cell("start-time", at), "display": cell("display-name", "Built-In Display"),
                         "surface-id": cell("uint64", 10),
                         "label": cell("narrative", f"Surface (viewer (20):Frame {slot}:Surface 10)")})
    tables = {
        "hitches": [], "hitches-frame-lifetimes": lifetimes, "displayed-surfaces-interval": surfaces,
        "time-info": [{"epoch": cell("mach-absolute-time", HOST), "timebase": cell("mach-timebase-info", "1/1")}],
    }
    return raw, tables


def hitch(start, duration, swap, process="viewer (20)"):
    return {"start": cell("start-time", start), "duration": cell("duration", duration),
            "display": cell("display-name", "Display 1"), "swap-id": cell("uint32", swap),
            "process": cell("process", process), "is-system": cell("boolean", 0)}


def observe_healthy(raw, tables):
    return summarize(raw, tables, "viewer", 20, "no_loss_reported")


class ObserveTest(unittest.TestCase):
    def test_selecting_only_hitches_and_display_keeps_instrument_provenance(self):
        log = Mock(spec=Path)
        log.is_file.return_value = False
        self.assertTrue(has_hitch_instruments("Animation Hitches", log))
        self.assertFalse(has_hitch_instruments("Time Profiler", log))
        self.assertFalse(has_hitch_instruments("Blank", log))
        self.assertTrue(has_hitch_instruments("Blank", log, selected_for_this_recording=True))
        log.is_file.return_value = True
        log.read_text.return_value = "Starting recording with the Blank template and Hitches, Display Instruments."
        self.assertTrue(has_hitch_instruments("Blank", log))

    def test_recorded_target_failure_survives_xctrace_exit_54(self):
        check_recording_end(54, "Target app exited", 1, 1)
        check_recording_end(54, "Target app exited", 2, 2)
        for status, reason, target, expected in [
            (54, "Time limit reached", 1, 1),
            (54, "Target app exited", 1, 0),
            (7, "Target app exited", 1, 1),
        ]:
            with self.assertRaises(ValueError):
                check_recording_end(status, reason, target, expected)

    def test_complete_empty_trace_and_missing_lifetime(self):
        raw, tables = evidence()
        self.assertTrue(observe_healthy(raw, tables)["complete_apple_frame_lifetime_coverage"])
        tables["hitches-frame-lifetimes"].pop(100)
        result = observe_healthy(raw, tables)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
        self.assertIsNone(result["hard_check_pass"])

    def test_hitch_time_is_clipped_to_the_active_interval(self):
        raw, tables = evidence()
        tables["hitches"].extend([
            hitch(START - 10 * PERIOD, 12 * PERIOD, 102),
            hitch(END - PERIOD, 2 * PERIOD, 1101),
        ])
        result = observe_healthy(raw, tables)
        self.assertFalse(result["hard_check_pass"])
        self.assertEqual(result["observed_hitch_time_ns"], 3 * PERIOD)
        self.assertEqual(result["observed_hitch_ms_per_second"], 3.0)

    def test_same_pid_alias_preserves_an_over_budget_hitch(self):
        raw, tables = evidence()
        for label in ["viewer (20)", "Oxiview Logs (20)"]:
            with self.subTest(label=label):
                tables["hitches"] = [hitch(START, 2 * PERIOD, 102, label)]
                result = observe_healthy(raw, tables)
                self.assertTrue(result["complete_apple_frame_lifetime_coverage"])
                self.assertEqual(result["observed_hitch_time_ns"], 20_000_000)
                self.assertEqual(result["observed_hitch_ms_per_second"], 2.0)
                self.assertIs(result["hard_check_pass"], False)

    def test_a_foreign_surface_hitch_is_excluded(self):
        raw, tables = evidence()
        at = START + 2 * PERIOD
        tables["displayed-surfaces-interval"].append({
            "start": cell("start-time", at), "display": cell("display-name", "Built-In Display"),
            "surface-id": cell("uint64", 11),
            "label": cell("narrative", "Surface (viewer (21):Frame 2:Surface 11)"),
        })
        tables["hitches-frame-lifetimes"].append({
            "start": cell("start-time", at - 1), "duration": cell("duration", 1),
            "display": cell("display-name", "Display 1"), "swap-id": cell("uint32", 9000),
            "surface-id": cell("uint32", 11),
        })
        other_display = hitch(START, 2 * PERIOD, 102, "viewer (21)")
        other_display["display"] = cell("display-name", "Display 2")
        tables["hitches"] = [hitch(START, 2 * PERIOD, 9000, "viewer (21)"), other_display]
        result = observe_healthy(raw, tables)
        self.assertEqual(result["observed_hitch_time_ns"], 0)
        self.assertEqual(result["unmatched_app_hitches"], 0)
        self.assertTrue(result["complete_apple_frame_lifetime_coverage"])

    def test_a_foreign_pid_on_the_app_swap_blocks_a_low_ratio(self):
        raw, tables = evidence()
        tables["hitches"] = [hitch(START, 2 * PERIOD, 102, "other (21)")]
        result = observe_healthy(raw, tables)
        self.assertEqual(result["apple_frame_lifetime_matches"], 1000)
        self.assertEqual(result["observed_hitch_time_ns"], 0)
        self.assertEqual(result["unmatched_app_hitches"], 1)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
        self.assertIsNone(result["hard_check_pass"])

    def test_unmatched_app_hitch_can_never_pass(self):
        raw, tables = evidence()
        tables["hitches"].append(hitch(START, PERIOD, 9000))
        result = observe_healthy(raw, tables)
        self.assertEqual(result["unmatched_app_hitches"], 1)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
        self.assertIsNone(result["hard_check_pass"])

    def test_missing_process_on_a_non_system_hitch_is_inconclusive(self):
        raw, tables = evidence()
        for owner in [cell("process", ""), cell("sentinel", "", "Unknown")]:
            with self.subTest(owner=owner.tag):
                row = hitch(START, 2 * PERIOD, 102)
                row["process"] = owner
                tables["hitches"] = [row]
                result = observe_healthy(raw, tables)
                self.assertEqual(result["unmatched_app_hitches"], 1)
                self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
                row["is-system"] = cell("boolean", 1)
                result = observe_healthy(raw, tables)
                self.assertEqual(result["unmatched_app_hitches"], 0)
                self.assertEqual(result["observed_hitch_time_ns"], 0)
                self.assertTrue(result["complete_apple_frame_lifetime_coverage"])

    def test_unparseable_or_conflicting_pid_on_an_app_swap_is_inconclusive(self):
        raw, tables = evidence()
        for label, structured in [
            ("viewer (unknown)", None),
            ("viewer (20)", "unreadable"),
            ("viewer (20)", "21"),
            ("other (21)", "20"),
        ]:
            with self.subTest(label=label, structured=structured):
                row = hitch(START, 2 * PERIOD, 102, label)
                if structured is not None:
                    row["process"].append(cell("pid", structured))
                tables["hitches"] = [row]
                result = observe_healthy(raw, tables)
                self.assertEqual(result["observed_hitch_time_ns"], 0)
                self.assertEqual(result["unmatched_app_hitches"], 1)
                self.assertFalse(result["complete_apple_frame_lifetime_coverage"])

    def test_apple_zero_cannot_certify_a_silent_metal_stall(self):
        raw, tables = evidence()
        del raw["frames"][5:10]
        raw.update(displayed_frames=995, dropped_frames=5)
        result = observe_healthy(raw, tables)
        self.assertTrue(result["complete_apple_frame_lifetime_coverage"])
        self.assertEqual(result["observed_hitch_time_ns"], 0)
        self.assertEqual(result["metal_missed_slot_ms_per_second"], 5.0)
        self.assertIsNone(result["hard_check_pass"])

    def test_unrelated_view_and_invalid_metal_capture_fail(self):
        raw, tables = evidence()
        result = summarize(raw, tables, "viewer", 21)
        self.assertEqual(result["apple_frame_lifetime_matches"], 0)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
        corrupted = dict(raw, pixel_width=100)
        with self.assertRaisesRegex(ValueError, "geometry"):
            summarize(corrupted, tables, "viewer", 20)
        corrupted = dict(raw, missing_feedback=1)
        with self.assertRaisesRegex(ValueError, "Metal feedback"):
            summarize(corrupted, tables, "viewer", 20)

    def test_xpath_layout_and_reference_are_checked(self):
        document = """<trace-query-result>
          <node xpath="/trace-toc/run[1]/data/table[1]"/>
          <node xpath="/trace-toc/run[1]/data/table[2]"/>
          <node xpath="/trace-toc/run[1]/data/table[3]"/>
          <node xpath="/trace-toc/run[1]/data/table[4]">
            <row><start-time/><mach-absolute-time id="a">1000</mach-absolute-time>
             <ignored/><mach-timebase-info fmt="1/1"/></row>
            <row><start-time/><mach-absolute-time ref="a"/>
             <ignored/><mach-timebase-info fmt="1/1"/></row>
          </node></trace-query-result>"""
        names = ["hitches", "hitches-frame-lifetimes", "displayed-surfaces-interval", "time-info"]
        parsed = read_tables([io.BytesIO(document.encode())], names)
        self.assertEqual(parsed["time-info"][1]["epoch"].text, "1000")
        wrong_type = document.replace("mach-absolute-time", "wall-clock")
        with self.assertRaisesRegex(ValueError, "Unexpected Xcode schema"):
            read_tables([io.BytesIO(wrong_type.encode())], names)

    def test_structured_owner_pid_and_its_xml_reference_identify_the_same_hitch(self):
        document = f"""<trace-query-result>
          <node xpath="/trace-toc/run[1]/data/table[1]">
            <row><start-time id="a">{START}</start-time><duration id="b">{2 * PERIOD}</duration>
              <process fmt="Oxiview Logs"><pid id="p">20</pid></process>
              <boolean id="c">0</boolean><uint32 id="d">102</uint32><ignored/>
              <display-name id="e" fmt="Display 1">Display 1</display-name></row>
            <row><start-time ref="a"/><duration ref="b"/>
              <process fmt="log-viewer-oxi"><pid ref="p"/></process>
              <boolean ref="c"/><uint32 ref="d"/><ignored/><display-name ref="e"/></row>
          </node>
          <node xpath="/trace-toc/run[1]/data/table[2]"/>
          <node xpath="/trace-toc/run[1]/data/table[3]"/>
          <node xpath="/trace-toc/run[1]/data/table[4]"/>
        </trace-query-result>"""
        raw, tables = evidence()
        tables["hitches"] = read_tables([io.BytesIO(document.encode())], list(tables))["hitches"]
        result = observe_healthy(raw, tables)
        self.assertEqual(result["observed_hitch_time_ns"], 20_000_000)
        self.assertEqual(len(result["correlated_app_hitches"]), 1)
        self.assertEqual(result["unmatched_app_hitches"], 0)
        self.assertTrue(result["complete_apple_frame_lifetime_coverage"])

        dangling = document.replace(
            '<process fmt="log-viewer-oxi"><pid ref="p"/></process>',
            '<process fmt="log-viewer-oxi (20)"><pid ref="missing"/></process>',
        )
        tables["hitches"] = read_tables([io.BytesIO(dangling.encode())], list(tables))["hitches"]
        result = observe_healthy(raw, tables)
        self.assertEqual(result["unmatched_app_hitches"], 1)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])

    def test_surface_tags_accept_both_names_for_the_app_pid(self):
        raw, tables = evidence()
        tables["displayed-surfaces-interval"][0]["label"] = cell(
            "narrative", "Surface (Oxiview Logs (20):Frame 0:Surface 10)")
        result = observe_healthy(raw, tables)
        self.assertEqual(result["app_tagged_surface_matches"], 1000)
        self.assertTrue(result["complete_apple_frame_lifetime_coverage"])

        tables["displayed-surfaces-interval"][0]["label"] = cell(
            "narrative", "Surface (Oxiview Logs (21):Frame 0:Surface 10)")
        result = observe_healthy(raw, tables)
        self.assertEqual(result["apple_frame_lifetime_matches"], 0)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])

    def test_guards_against_wrong_display_surface_and_app_pid(self):
        for field, tag, value in [
            ("display", "display-name", "external-1"),
            ("surface-id", "uint64", 11),
            ("label", "narrative", "Surface (viewer (21):Frame 1:Surface 10)"),
        ]:
            with self.subTest(field=field):
                raw, tables = evidence()
                for row in tables["displayed-surfaces-interval"]:
                    row[field] = cell(tag, value)
                self.assertFalse(observe_healthy(raw, tables)["complete_apple_frame_lifetime_coverage"])

    def test_trace_identifies_the_staged_bundle_among_same_named_executables(self):
        run = ET.Element("run")
        processes = ET.SubElement(run, "processes")
        selected = Path("/capture/Oxiview Logs.app/Contents/MacOS/log-viewer-oxi")
        other = Path("/previous/Oxiview Logs.app/Contents/MacOS/log-viewer-oxi")
        target = ET.Element("process", pid="42", name=selected.name)
        ET.SubElement(processes, "process", pid="41", path=str(other))
        ET.SubElement(processes, "process", pid="42", path=str(selected))
        ET.SubElement(processes, "process", pid="42", path=str(selected))
        self.assertEqual(traced_executable(run, target, selected), str(selected))
        with self.assertRaisesRegex(ValueError, "disagrees"):
            traced_executable(run, target, other)
        target.set("name", "Oxiview Logs")
        self.assertEqual(traced_executable(run, target, selected), str(selected))
        ET.SubElement(processes, "process", pid="42", path=str(other))
        with self.assertRaisesRegex(ValueError, "conflicting"):
            traced_executable(run, target, selected)

    def test_trace_requires_the_launched_pids_path_for_fresh_recordings(self):
        run = ET.Element("run")
        processes = ET.SubElement(run, "processes")
        selected = Path("/capture/Oxiview Logs.app/Contents/MacOS/log-viewer-oxi")
        target = ET.Element("process", pid="42", name=selected.name)
        ET.SubElement(processes, "process", pid="41", path=str(selected))
        ET.SubElement(processes, "process", pid="42", name=selected.name)
        self.assertIsNone(traced_executable(run, target, None))
        with self.assertRaisesRegex(ValueError, "no executable path"):
            traced_executable(run, target, selected)

    def test_a_duplicate_lifetime_swap_cannot_cover_distinct_presents(self):
        raw, tables = evidence()
        tables["hitches-frame-lifetimes"][100]["swap-id"] = cell("uint32", 201)
        result = observe_healthy(raw, tables)
        self.assertEqual(result["conflicting_apple_lifetimes"], 1)
        self.assertEqual(result["apple_frame_lifetime_matches"], 998)
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])

    def test_one_cluster_of_viewer_tags_cannot_certify_the_whole_sample(self):
        raw, tables = evidence()
        tables["displayed-surfaces-interval"] = tables["displayed-surfaces-interval"][:100]
        result = observe_healthy(raw, tables)
        self.assertEqual(result["apple_frame_lifetime_matches"], 1000)
        self.assertEqual(result["app_tagged_surface_matches"], 100)
        self.assertFalse(result["app_surface_matches_bracket_sample"])
        self.assertFalse(result["complete_apple_frame_lifetime_coverage"])

    def test_duplicate_intervals_are_counted_once_and_conflicts_are_preserved(self):
        raw, tables = evidence()
        interval = hitch(START, 2 * PERIOD, 102)
        tables["hitches"].extend([interval, interval, hitch(START, 2 * PERIOD, 102, "Oxiview Logs (20)")])
        result = observe_healthy(raw, tables)
        self.assertEqual(result["observed_hitch_time_ns"], 2 * PERIOD)
        self.assertEqual(len(result["correlated_app_hitches"]), 1)
        tables["hitches"].append(hitch(START - 1, 2 * PERIOD + 1, 102))
        result = observe_healthy(raw, tables)
        self.assertEqual(result["observed_hitch_time_ns"], 0)
        self.assertEqual(result["unmatched_app_hitches"], 2)

    def test_unavailable_or_loss_reported_stream_is_inconclusive_when_quiet(self):
        raw, tables = evidence()
        self.assertEqual(trace_issues(Path("nonexistent.trace")), ("unavailable", []))
        for health in ["unavailable", "loss_reported"]:
            result = summarize(raw, tables, "viewer", 20, health)
            self.assertFalse(result["complete_apple_frame_lifetime_coverage"])
            self.assertIsNone(result["hard_check_pass"])

    def test_trace_issue_count_is_preserved(self):
        with sqlite3.connect(":memory:") as db:
            db.execute("CREATE TABLE ZISSUE (ZRELATIVETIMESTAMP INTEGER, ZCOUNT INTEGER, ZMESSAGE TEXT)")
            db.execute("INSERT INTO ZISSUE VALUES (?, ?, ?)", (START, 25, "Kdebug dropped 25 records"))
            with patch.object(Path, "is_file", return_value=True), patch("observe.sqlite3.connect", return_value=db):
                health, issues = trace_issues(Path("recorded.trace"))
        self.assertEqual(health, "loss_reported")
        self.assertEqual(issues, [{"trace_time_ns": START, "count": 25, "message": "Kdebug dropped 25 records"}])


if __name__ == "__main__":
    unittest.main()
