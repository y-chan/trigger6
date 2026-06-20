#!/usr/bin/env python3
"""Inspect Trigger6 type 7 JPEG tile updates.

The tool accepts either:

- a CSV previously generated from type 7 video headers, such as
  captures/1080p_video_headers.csv
- a pcap/pcapng file, parsed through tools/t6_pcap_summary.py helpers

For CSV input, interrupt/fence rows are unavailable, but tile grouping and
address patterns can still be inspected. For pcap input, the timeline also
prints nearby interrupt rows.
"""

from __future__ import annotations

import argparse
import base64
import csv
import json
import pathlib
import sys
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import t6_pcap_summary as pcap_summary  # noqa: E402


BULK_COMMAND_LEN = 32


@dataclass
class Type7Row:
    pcap: str
    frame: int
    time: float
    cmd_frame: int
    session: int
    cmd_dest: int
    cmd_total_len: int
    payload_len: int
    data_len: int
    sequence: int
    hint: int
    width: int
    height: int
    canvas_width: int
    canvas_height: int
    start_addr: int
    end_addr: int
    image_format: int
    jpeg_width: int | None
    jpeg_height: int | None
    sof_marker: int | None = None
    jpeg_precision: int | None = None
    jpeg_components: str | None = None
    payload_b64: str | None = None

    @property
    def vram_span(self) -> int:
        return self.end_addr - self.start_addr

    @property
    def jpeg_size(self) -> str:
        if self.jpeg_width is None or self.jpeg_height is None:
            return "?"
        return f"{self.jpeg_width}x{self.jpeg_height}"


@dataclass
class InterruptRow:
    frame: int
    time: float
    flags: int
    value: int
    event: int


def parse_jpeg_details(data: bytes, soi_offset: int) -> tuple[int | None, int | None, str | None]:
    if soi_offset < 0 or soi_offset + 4 >= len(data):
        return None, None, None

    pos = soi_offset + 2
    while pos + 4 <= len(data):
        if data[pos] != 0xFF:
            pos += 1
            continue
        while pos < len(data) and data[pos] == 0xFF:
            pos += 1
        if pos >= len(data):
            break
        marker = data[pos]
        pos += 1
        if marker == 0xDA:
            break
        if marker in {0xD8, 0xD9}:
            continue
        if pos + 2 > len(data):
            break
        seg_len = int.from_bytes(data[pos : pos + 2], "big")
        if seg_len < 2 or pos + seg_len > len(data):
            break
        if marker in {
            0xC0,
            0xC1,
            0xC2,
            0xC3,
            0xC5,
            0xC6,
            0xC7,
            0xC9,
            0xCA,
            0xCB,
            0xCD,
            0xCE,
            0xCF,
        }:
            if seg_len < 8:
                break
            precision = data[pos + 2]
            comp_count = data[pos + 7]
            comp_pos = pos + 8
            comps: list[str] = []
            for _ in range(comp_count):
                if comp_pos + 3 > pos + seg_len:
                    break
                comp_id = data[comp_pos]
                sampling = data[comp_pos + 1]
                qtable = data[comp_pos + 2]
                comps.append(
                    f"id{comp_id}:{sampling >> 4}x{sampling & 0x0f}:q{qtable}"
                )
                comp_pos += 3
            return marker, precision, ",".join(comps)
        pos += seg_len
    return None, None, None


def parse_int(value: str | int | None) -> int:
    if value is None or value == "":
        return 0
    if isinstance(value, int):
        return value
    return int(value, 0)


def parse_optional_int(value: str | None) -> int | None:
    if value is None or value == "":
        return None
    return int(value, 0)


def load_csv(path: pathlib.Path) -> tuple[list[Type7Row], list[InterruptRow]]:
    rows: list[Type7Row] = []
    with path.open(newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            if parse_int(row.get("video_type")) != 7:
                continue
            rows.append(
                Type7Row(
                    pcap=row.get("pcap") or path.name,
                    frame=parse_int(row.get("frame")),
                    time=float(row.get("time") or 0),
                    cmd_frame=parse_int(row.get("cmd_frame")),
                    session=parse_int(row.get("session")),
                    cmd_dest=parse_int(row.get("cmd_dest")),
                    cmd_total_len=parse_int(row.get("cmd_total_len")),
                    payload_len=parse_int(row.get("payload_len")),
                    data_len=parse_int(row.get("data_len")),
                    sequence=parse_int(row.get("sequence")),
                    hint=parse_int(row.get("hint")),
                    width=parse_int(row.get("width_field")),
                    height=parse_int(row.get("height_field")),
                    canvas_width=parse_int(row.get("canvas_width")),
                    canvas_height=parse_int(row.get("canvas_height")),
                    start_addr=parse_int(row.get("start_addr")),
                    end_addr=parse_int(row.get("end_addr")),
                    image_format=parse_int(row.get("image_format")),
                    jpeg_width=parse_optional_int(row.get("jpeg_width")),
                    jpeg_height=parse_optional_int(row.get("jpeg_height")),
                )
            )
    return rows, []


def load_pcap(path: pathlib.Path, tshark: str | None) -> tuple[list[Type7Row], list[InterruptRow]]:
    packets = pcap_summary.load_packets(str(path), pcap_summary.tshark_path(tshark))
    type7_rows: list[Type7Row] = []
    interrupts: list[InterruptRow] = []
    pending: pcap_summary.BulkCommand | None = None

    for packet in packets:
        if packet.endpoint == pcap_summary.DEFAULT_BULK_OUT and packet.capdata:
            command = pcap_summary.parse_bulk_command(packet)
            if command is not None:
                pending = command
                continue
            if pending is not None:
                header = pcap_summary.parse_video_header(packet, pending)
                if header is not None and header.video_type == 7:
                    type7_rows.append(
                        Type7Row(
                            pcap=path.name,
                            frame=header.frame,
                            time=float(header.time or 0),
                            cmd_frame=header.command_frame,
                            session=header.session,
                            cmd_dest=pending.dest,
                            cmd_total_len=pending.total_len,
                            payload_len=header.payload_len,
                            data_len=header.data_len,
                            sequence=header.sequence,
                            hint=header.flags_or_format_hint,
                            width=header.width_field,
                            height=header.height_field,
                            canvas_width=header.canvas_width or 0,
                            canvas_height=header.canvas_height or 0,
                            start_addr=header.start_addr,
                            end_addr=header.end_addr,
                            image_format=header.image_format,
                            jpeg_width=header.jpeg_width,
                            jpeg_height=header.jpeg_height,
                            sof_marker=parse_jpeg_details(
                                packet.capdata, header.jpeg_soi_offset
                            )[0],
                            jpeg_precision=parse_jpeg_details(
                                packet.capdata, header.jpeg_soi_offset
                            )[1],
                            jpeg_components=parse_jpeg_details(
                                packet.capdata, header.jpeg_soi_offset
                            )[2],
                            payload_b64=base64.b64encode(packet.capdata).decode("ascii"),
                        )
                    )
                pending = None

        if (
            packet.endpoint == pcap_summary.DEFAULT_INTERRUPT_IN
            and packet.data_len == 64
            and len(packet.capdata) >= 0x14
        ):
            interrupts.append(
                InterruptRow(
                    frame=packet.frame,
                    time=float(packet.time or 0),
                    flags=packet.capdata[0],
                    value=int.from_bytes(packet.capdata[0x0C:0x10], "little"),
                    event=packet.capdata[0x13],
                )
            )

    return type7_rows, interrupts


def group_rows(rows: list[Type7Row], max_gap_s: float) -> list[list[Type7Row]]:
    if not rows:
        return []
    groups: list[list[Type7Row]] = [[rows[0]]]
    for row in rows[1:]:
        prev = groups[-1][-1]
        same_stream = row.pcap == prev.pcap and row.session == prev.session
        seq_delta = row.sequence - prev.sequence
        if same_stream and row.time - prev.time <= max_gap_s and 0 < seq_delta <= 4:
            groups[-1].append(row)
        else:
            groups.append([row])
    return groups


def nearest_interrupts(
    interrupts: list[InterruptRow], start_time: float, end_time: float, window_s: float
) -> list[InterruptRow]:
    return [
        row
        for row in interrupts
        if start_time - window_s <= row.time <= end_time + window_s
    ]


def print_group(
    index: int,
    group: list[Type7Row],
    interrupts: list[InterruptRow],
    interrupt_window_s: float,
    verbose: bool,
) -> None:
    start = group[0]
    end = group[-1]
    total_payload = sum(row.payload_len for row in group)
    total_jpeg = sum(max(0, row.data_len - pcap_summary.VIDEO_HEADER_LEN) for row in group)
    start_addrs = sorted({row.start_addr for row in group})
    end_addrs = sorted({row.end_addr for row in group})
    cmd_dests = sorted({row.cmd_dest for row in group})
    spans = sorted({row.vram_span for row in group})
    sizes = ",".join(f"{row.width}x{row.height}" for row in group[:8])
    if len(group) > 8:
        sizes += ",..."

    print(
        "group\t"
        f"idx={index}\t"
        f"count={len(group)}\t"
        f"time={start.time:.6f}-{end.time:.6f}\t"
        f"pcap={start.pcap}\t"
        f"session={start.session}\t"
        f"frames={start.frame}-{end.frame}\t"
        f"seq={start.sequence}-{end.sequence}\t"
        f"payload={total_payload}\t"
        f"jpeg_bytes~={total_jpeg}\t"
        f"cmd_dests={','.join(hex(v) for v in cmd_dests[:4])}\t"
        f"starts={','.join(hex(v) for v in start_addrs[:4])}\t"
        f"ends={','.join(hex(v) for v in end_addrs[:4])}\t"
        f"spans={','.join(hex(v) for v in spans[:4])}\t"
        f"sizes={sizes}"
    )

    nearby_interrupts = nearest_interrupts(
        interrupts, start.time, end.time, interrupt_window_s
    )
    for intr in nearby_interrupts[:12]:
        print(
            "  interrupt\t"
            f"frame={intr.frame}\t"
            f"time={intr.time:.6f}\t"
            f"dt_start={(intr.time - start.time) * 1000:.2f}ms\t"
            f"flags=0x{intr.flags:02x}\t"
            f"value=0x{intr.value:08x}\t"
            f"event=0x{intr.event:02x}"
        )
    if len(nearby_interrupts) > 12:
        print(f"  interrupt_more\tcount={len(nearby_interrupts) - 12}")

    if verbose:
        for row in group:
            print(
                "  tile\t"
                f"frame={row.frame}\t"
                f"cmd_frame={row.cmd_frame}\t"
                f"pcap={row.pcap}\t"
                f"session={row.session}\t"
                f"time={row.time:.6f}\t"
                f"seq={row.sequence}\t"
                f"cmd_dest=0x{row.cmd_dest:08x}\t"
                f"payload={row.payload_len}\t"
                f"data_len={row.data_len}\t"
                f"size={row.width}x{row.height}\t"
                f"jpeg={row.jpeg_size}\t"
                f"canvas={row.canvas_width}x{row.canvas_height}\t"
                f"start=0x{row.start_addr:x}\t"
                f"end=0x{row.end_addr:x}\t"
                f"span=0x{row.vram_span:x}\t"
                f"format=0x{row.image_format:x}"
            )


def print_address_summary(rows: list[Type7Row]) -> None:
    by_pair: dict[tuple[int, int], list[Type7Row]] = {}
    for row in rows:
        by_pair.setdefault((row.start_addr, row.end_addr), []).append(row)

    print("# address_pairs")
    for (start, end), pair_rows in sorted(
        by_pair.items(), key=lambda item: (-len(item[1]), item[0][0])
    )[:20]:
        sizes = sorted({(row.width, row.height) for row in pair_rows})
        size_s = ",".join(f"{w}x{h}" for w, h in sizes[:8])
        if len(sizes) > 8:
            size_s += ",..."
        print(
            "addr_pair\t"
            f"count={len(pair_rows)}\t"
            f"start=0x{start:x}\t"
            f"end=0x{end:x}\t"
            f"span=0x{end - start:x}\t"
            f"sizes={size_s}"
        )


def print_ack_summary(
    rows: list[Type7Row], interrupts: list[InterruptRow], window_s: float
) -> None:
    if not interrupts:
        print("# ack_summary")
        print("ack_summary\tinterrupts=0")
        return

    by_value: dict[int, list[InterruptRow]] = {}
    for intr in interrupts:
        if intr.flags == 0x04 and intr.event == 0x04:
            by_value.setdefault(intr.value, []).append(intr)

    matched: list[tuple[Type7Row, InterruptRow, float]] = []
    missing: list[Type7Row] = []
    negative: list[tuple[Type7Row, InterruptRow, float]] = []
    for row in rows:
        candidates = [
            intr
            for intr in by_value.get(row.sequence, [])
            if abs(intr.time - row.time) <= window_s
        ]
        if not candidates:
            missing.append(row)
            continue
        intr = min(candidates, key=lambda item: abs(item.time - row.time))
        dt_ms = (intr.time - row.time) * 1000.0
        matched.append((row, intr, dt_ms))
        if dt_ms < 0:
            negative.append((row, intr, dt_ms))

    dts = sorted(dt for _, _, dt in matched)

    print("# ack_summary")
    print(
        "ack_summary\t"
        f"tiles={len(rows)}\t"
        f"interrupt_values={len(by_value)}\t"
        f"matched={len(matched)}\t"
        f"missing={len(missing)}\t"
        f"negative_dt={len(negative)}"
    )
    if dts:
        print(
            "ack_latency_ms\t"
            f"min={dts[0]:.3f}\t"
            f"p50={dts[len(dts) // 2]:.3f}\t"
            f"p90={dts[int(len(dts) * 0.9)]:.3f}\t"
            f"max={dts[-1]:.3f}"
        )

    by_count: dict[int, int] = {}
    for row, _, _ in matched:
        by_count[row.sequence] = by_count.get(row.sequence, 0) + 1
    duplicate_matches = sum(1 for count in by_count.values() if count > 1)
    print(f"ack_duplicate_sequence_matches\t{duplicate_matches}")

    for row, intr, dt_ms in matched[:20]:
        print(
            "ack_match\t"
            f"tile_frame={row.frame}\t"
            f"seq=0x{row.sequence:08x}\t"
            f"tile_time={row.time:.6f}\t"
            f"interrupt_frame={intr.frame}\t"
            f"interrupt_time={intr.time:.6f}\t"
            f"dt_ms={dt_ms:.3f}\t"
            f"size={row.width}x{row.height}\t"
            f"cmd_dest=0x{row.cmd_dest:08x}\t"
            f"start=0x{row.start_addr:x}\t"
            f"end=0x{row.end_addr:x}"
        )
    if len(matched) > 20:
        print(f"ack_match_more\t{len(matched) - 20}")

    for row in missing[:20]:
        print(
            "ack_missing\t"
            f"tile_frame={row.frame}\t"
            f"seq=0x{row.sequence:08x}\t"
            f"time={row.time:.6f}\t"
            f"size={row.width}x{row.height}\t"
            f"cmd_dest=0x{row.cmd_dest:08x}"
        )
    if len(missing) > 20:
        print(f"ack_missing_more\t{len(missing) - 20}")


def print_cmd_dest_summary(rows: list[Type7Row]) -> None:
    if not rows:
        return

    print("# cmd_dest_summary")
    prev: Type7Row | None = None
    deltas: dict[int, int] = {}
    wraps = 0
    for row in rows:
        if prev is not None and row.pcap == prev.pcap and row.session == prev.session:
            delta = row.cmd_dest - prev.cmd_dest
            if delta < 0:
                wraps += 1
            else:
                deltas[delta] = deltas.get(delta, 0) + 1
        prev = row

    common = sorted(deltas.items(), key=lambda item: (-item[1], item[0]))[:20]
    print(
        "cmd_dest_summary\t"
        f"rows={len(rows)}\t"
        f"wraps={wraps}\t"
        f"min=0x{min(row.cmd_dest for row in rows):x}\t"
        f"max=0x{max(row.cmd_dest for row in rows):x}"
    )
    for delta, count in common:
        print(f"cmd_dest_delta\tcount={count}\tdelta=0x{delta:x}")


def align_up(value: int, alignment: int) -> int:
    return (value + alignment - 1) // alignment * alignment


def print_cmd_dest_payload_correlation(rows: list[Type7Row]) -> None:
    if not rows:
        return

    print("# cmd_dest_payload_correlation")
    total = 0
    wraps = 0
    exact_payload = 0
    exact_payload_minus_cmd = 0
    exact_cmd_total = 0
    exact_cmd_total_minus_cmd = 0
    exact_data = 0
    close_payload = 0
    consecutive_total = 0
    consecutive_exact_payload_minus_cmd = 0
    samples: list[tuple[Type7Row, Type7Row, int, int, int, int, int]] = []

    prev: Type7Row | None = None
    for row in rows:
        if prev is None or row.pcap != prev.pcap or row.session != prev.session:
            prev = row
            continue

        delta = row.cmd_dest - prev.cmd_dest
        if delta < 0:
            wraps += 1
            prev = row
            continue

        total += 1
        payload_aligned = align_up(prev.payload_len, 1024)
        cmd_total_aligned = align_up(prev.cmd_total_len, 1024)
        data_aligned = align_up(prev.data_len, 1024)

        if delta == payload_aligned:
            exact_payload += 1
        if delta == payload_aligned - BULK_COMMAND_LEN:
            exact_payload_minus_cmd += 1
        if delta == cmd_total_aligned:
            exact_cmd_total += 1
        if delta == cmd_total_aligned - BULK_COMMAND_LEN:
            exact_cmd_total_minus_cmd += 1
        if delta == data_aligned:
            exact_data += 1
        if abs(delta - payload_aligned) <= 1024:
            close_payload += 1
        if row.sequence == prev.sequence + 1:
            consecutive_total += 1
            if delta == payload_aligned - BULK_COMMAND_LEN:
                consecutive_exact_payload_minus_cmd += 1

        if len(samples) < 30:
            samples.append(
                (
                    prev,
                    row,
                    delta,
                    payload_aligned,
                    payload_aligned - BULK_COMMAND_LEN,
                    cmd_total_aligned,
                    data_aligned,
                )
            )
        prev = row

    print(
        "cmd_dest_payload_correlation\t"
        f"pairs={total}\t"
        f"wraps={wraps}\t"
        f"exact_align_payload={exact_payload}\t"
        f"exact_align_payload_minus_32={exact_payload_minus_cmd}\t"
        f"exact_align_cmd_total={exact_cmd_total}\t"
        f"exact_align_cmd_total_minus_32={exact_cmd_total_minus_cmd}\t"
        f"exact_align_data_len={exact_data}\t"
        f"within_1024_payload={close_payload}\t"
        f"consecutive_pairs={consecutive_total}\t"
        f"consecutive_exact_align_payload_minus_32={consecutive_exact_payload_minus_cmd}"
    )
    for (
        prev_row,
        row,
        delta,
        payload_aligned,
        payload_aligned_minus_cmd,
        cmd_total_aligned,
        data_aligned,
    ) in samples:
        print(
            "cmd_dest_step\t"
            f"prev_frame={prev_row.frame}\t"
            f"next_frame={row.frame}\t"
            f"prev_seq=0x{prev_row.sequence:08x}\t"
            f"next_seq=0x{row.sequence:08x}\t"
            f"delta=0x{delta:x}\t"
            f"align_payload=0x{payload_aligned:x}\t"
            f"align_payload_minus_32=0x{payload_aligned_minus_cmd:x}\t"
            f"align_cmd_total=0x{cmd_total_aligned:x}\t"
            f"align_data=0x{data_aligned:x}\t"
            f"prev_payload={prev_row.payload_len}\t"
            f"prev_cmd_total={prev_row.cmd_total_len}\t"
            f"prev_size={prev_row.width}x{prev_row.height}"
        )


def print_address_transition_summary(rows: list[Type7Row]) -> None:
    if not rows:
        return

    print("# address_transition_summary")
    by_pair: dict[tuple[int, int], list[Type7Row]] = {}
    transitions: dict[tuple[tuple[int, int], tuple[int, int]], int] = {}
    same_pair_steps = 0
    pair_changes = 0
    consecutive_same_pair = 0
    consecutive_pair_changes = 0

    prev: Type7Row | None = None
    for row in rows:
        pair = (row.start_addr, row.end_addr)
        by_pair.setdefault(pair, []).append(row)
        if prev is not None and row.pcap == prev.pcap and row.session == prev.session:
            prev_pair = (prev.start_addr, prev.end_addr)
            transition = (prev_pair, pair)
            transitions[transition] = transitions.get(transition, 0) + 1
            if pair == prev_pair:
                same_pair_steps += 1
            else:
                pair_changes += 1
            if row.sequence == prev.sequence + 1:
                if pair == prev_pair:
                    consecutive_same_pair += 1
                else:
                    consecutive_pair_changes += 1
        prev = row

    print(
        "address_transition_summary\t"
        f"pairs={len(by_pair)}\t"
        f"steps={same_pair_steps + pair_changes}\t"
        f"same_pair_steps={same_pair_steps}\t"
        f"pair_changes={pair_changes}\t"
        f"consecutive_same_pair={consecutive_same_pair}\t"
        f"consecutive_pair_changes={consecutive_pair_changes}"
    )

    print("# address_pair_runs")
    for (start, end), pair_rows in sorted(
        by_pair.items(), key=lambda item: (-len(item[1]), item[0][0])
    )[:30]:
        seqs = [row.sequence for row in pair_rows]
        cmd_dests = [row.cmd_dest for row in pair_rows]
        sizes = sorted({(row.width, row.height) for row in pair_rows})
        pcaps = sorted({row.pcap for row in pair_rows})
        size_s = ",".join(f"{w}x{h}" for w, h in sizes[:8])
        if len(sizes) > 8:
            size_s += ",..."
        print(
            "address_pair\t"
            f"count={len(pair_rows)}\t"
            f"start=0x{start:x}\t"
            f"end=0x{end:x}\t"
            f"span=0x{end - start:x}\t"
            f"seq_min=0x{min(seqs):x}\t"
            f"seq_max=0x{max(seqs):x}\t"
            f"cmd_min=0x{min(cmd_dests):x}\t"
            f"cmd_max=0x{max(cmd_dests):x}\t"
            f"pcaps={','.join(pcaps[:3])}\t"
            f"sizes={size_s}"
        )

    print("# address_transitions")
    for ((from_start, from_end), (to_start, to_end)), count in sorted(
        transitions.items(), key=lambda item: (-item[1], item[0])
    )[:40]:
        print(
            "address_transition\t"
            f"count={count}\t"
            f"from=0x{from_start:x}-0x{from_end:x}\t"
            f"to=0x{to_start:x}-0x{to_end:x}\t"
            f"same={from_start == to_start and from_end == to_end}"
        )


def print_jpeg_summary(rows: list[Type7Row]) -> None:
    print("# jpeg_summary")
    detail_rows = [row for row in rows if row.sof_marker is not None]
    if not detail_rows:
        print("jpeg_summary\trows_with_details=0")
        return

    by_sof: dict[int, int] = {}
    by_components: dict[str, int] = {}
    by_size_components: dict[tuple[int, int, str], int] = {}
    for row in detail_rows:
        assert row.sof_marker is not None
        components = row.jpeg_components or "?"
        by_sof[row.sof_marker] = by_sof.get(row.sof_marker, 0) + 1
        by_components[components] = by_components.get(components, 0) + 1
        by_size_components[(row.width, row.height, components)] = (
            by_size_components.get((row.width, row.height, components), 0) + 1
        )

    print(
        "jpeg_summary\t"
        f"rows={len(rows)}\t"
        f"rows_with_details={len(detail_rows)}"
    )
    for marker, count in sorted(by_sof.items(), key=lambda item: (-item[1], item[0])):
        marker_name = {
            0xC0: "baseline",
            0xC1: "extended-sequential",
            0xC2: "progressive",
        }.get(marker, "sof")
        print(f"jpeg_sof\tcount={count}\tmarker=0x{marker:02x}\tname={marker_name}")
    for components, count in sorted(
        by_components.items(), key=lambda item: (-item[1], item[0])
    ):
        print(f"jpeg_components\tcount={count}\tcomponents={components}")
    for (width, height, components), count in sorted(
        by_size_components.items(), key=lambda item: (-item[1], item[0])
    )[:40]:
        print(
            "jpeg_size_components\t"
            f"count={count}\t"
            f"size={width}x{height}\t"
            f"components={components}"
        )


def export_groups_json(
    path: pathlib.Path,
    groups: list[list[Type7Row]],
    interrupts: list[InterruptRow],
    limit_groups: int,
    interrupt_window_s: float,
) -> None:
    out_groups = []
    selected = groups if limit_groups <= 0 else groups[:limit_groups]
    for index, group in enumerate(selected, start=1):
        nearby = nearest_interrupts(
            interrupts, group[0].time, group[-1].time, interrupt_window_s
        )
        out_groups.append(
            {
                "index": index,
                "pcap": group[0].pcap,
                "session": group[0].session,
                "time_start": group[0].time,
                "time_end": group[-1].time,
                "tiles": [
                    {
                        "frame": row.frame,
                        "cmd_frame": row.cmd_frame,
                        "time": row.time,
                        "sequence": row.sequence,
                        "cmd_dest": row.cmd_dest,
                        "cmd_total_len": row.cmd_total_len,
                        "payload_len": row.payload_len,
                        "data_len": row.data_len,
                        "hint": row.hint,
                        "width": row.width,
                        "height": row.height,
                        "canvas_width": row.canvas_width,
                        "canvas_height": row.canvas_height,
                        "start_addr": row.start_addr,
                        "end_addr": row.end_addr,
                        "image_format": row.image_format,
                        "jpeg_width": row.jpeg_width,
                        "jpeg_height": row.jpeg_height,
                        "sof_marker": row.sof_marker,
                        "jpeg_precision": row.jpeg_precision,
                        "jpeg_components": row.jpeg_components,
                        "payload_b64": row.payload_b64,
                    }
                    for row in group
                ],
                "interrupts": [
                    {
                        "frame": intr.frame,
                        "time": intr.time,
                        "flags": intr.flags,
                        "value": intr.value,
                        "event": intr.event,
                    }
                    for intr in nearby
                ],
            }
        )

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"groups": out_groups}, indent=2), encoding="utf-8")
    print(f"exported_groups_json\t{path}\tgroups={len(out_groups)}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", help="captures/1080p_video_headers.csv or pcap/pcapng")
    parser.add_argument("--tshark", help="path to tshark for pcap input")
    parser.add_argument("--max-gap-ms", type=float, default=5.0)
    parser.add_argument("--interrupt-window-ms", type=float, default=8.0)
    parser.add_argument("--limit-groups", type=int, default=20)
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--address-summary", action="store_true")
    parser.add_argument("--ack-summary", action="store_true")
    parser.add_argument("--cmd-dest-summary", action="store_true")
    parser.add_argument("--cmd-dest-payload-correlation", action="store_true")
    parser.add_argument("--address-transition-summary", action="store_true")
    parser.add_argument("--jpeg-summary", action="store_true")
    parser.add_argument("--export-groups-json", type=pathlib.Path)
    args = parser.parse_args()

    path = pathlib.Path(args.input)
    if path.suffix.lower() == ".csv":
        rows, interrupts = load_csv(path)
    else:
        rows, interrupts = load_pcap(path, args.tshark)

    rows.sort(key=lambda row: (row.time, row.frame))
    groups = group_rows(rows, args.max_gap_ms / 1000.0)

    print("# summary")
    print(f"type7_rows\t{len(rows)}")
    print(f"groups\t{len(groups)}")
    print(f"interrupts\t{len(interrupts)}")
    if rows:
        print(f"time_range\t{rows[0].time:.6f}-{rows[-1].time:.6f}")
    print()

    if args.address_summary:
        print_address_summary(rows)
        print()

    if args.cmd_dest_summary:
        print_cmd_dest_summary(rows)
        print()

    if args.cmd_dest_payload_correlation:
        print_cmd_dest_payload_correlation(rows)
        print()

    if args.address_transition_summary:
        print_address_transition_summary(rows)
        print()

    if args.jpeg_summary:
        print_jpeg_summary(rows)
        print()

    if args.export_groups_json:
        export_groups_json(
            args.export_groups_json,
            groups,
            interrupts,
            args.limit_groups,
            args.interrupt_window_ms / 1000.0,
        )
        print()

    if args.ack_summary:
        print_ack_summary(rows, interrupts, args.interrupt_window_ms / 1000.0)
        print()

    for index, group in enumerate(groups[: args.limit_groups], start=1):
        print_group(
            index,
            group,
            interrupts,
            args.interrupt_window_ms / 1000.0,
            args.verbose,
        )

    if len(groups) > args.limit_groups:
        print(f"# omitted_groups\t{len(groups) - args.limit_groups}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
