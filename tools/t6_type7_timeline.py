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
import csv
import pathlib
import sys
from dataclasses import dataclass


ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import t6_pcap_summary as pcap_summary  # noqa: E402


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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", help="captures/1080p_video_headers.csv or pcap/pcapng")
    parser.add_argument("--tshark", help="path to tshark for pcap input")
    parser.add_argument("--max-gap-ms", type=float, default=5.0)
    parser.add_argument("--interrupt-window-ms", type=float, default=8.0)
    parser.add_argument("--limit-groups", type=int, default=20)
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--address-summary", action="store_true")
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
