#!/usr/bin/env python3
"""Reassemble fragmented Trigger6 display bulk transfers and inspect video payloads."""

from __future__ import annotations

import argparse
import collections
import csv
import html
import json
import pathlib
import re

from t6_pcap_summary import (
    DEFAULT_BULK_OUT,
    BulkCommand,
    UsbPacket,
    load_packets,
    parse_bulk_command,
    parse_video_header,
    tshark_path,
)


class Assembly:
    def __init__(self, command: BulkCommand) -> None:
        self.first_command = command
        self.last_command = command
        self.data = bytearray(command.total_len)
        self.received = bytearray(command.total_len)
        self.fragments = 0

    def add(self, command: BulkCommand, payload: bytes) -> None:
        end = command.fragment_offset + len(payload)
        if end > command.total_len:
            raise ValueError(
                f"fragment exceeds total: off=0x{command.fragment_offset:x} "
                f"len=0x{len(payload):x} total=0x{command.total_len:x}"
            )
        self.data[command.fragment_offset:end] = payload
        self.received[command.fragment_offset:end] = b"\x01" * len(payload)
        self.last_command = command
        self.fragments += 1

    @property
    def complete(self) -> bool:
        return all(self.received)

    @property
    def received_len(self) -> int:
        return sum(1 for b in self.received if b)


def jpeg_eoi_offset(data: bytes, soi: int) -> int | None:
    if soi < 0:
        return None
    pos = data.find(b"\xff\xd9", soi + 2)
    return None if pos < 0 else pos + 2


def trim_trailing_zeroes(data: bytes) -> bytes:
    last = len(data) - 1
    while last >= 0 and data[last] == 0:
        last -= 1
    return data[: last + 1]


def safe_stem(path: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", pathlib.Path(path).stem)


def summarize_reassembled(args: argparse.Namespace) -> int:
    packets = load_packets(args.pcap, tshark_path(args.tshark))
    pending_command: BulkCommand | None = None
    assemblies: dict[tuple[int, int, int], Assembly] = {}
    complete: list[Assembly] = []
    malformed = 0

    for packet in packets:
        if packet.endpoint != args.bulk_out or not packet.capdata:
            continue

        command = parse_bulk_command(packet)
        if command is not None:
            pending_command = command
            continue

        if pending_command is None:
            continue

        command = pending_command
        pending_command = None
        key = (command.session, command.dest, command.total_len)
        assembly = assemblies.get(key)
        if assembly is None or command.fragment_offset == 0:
            assembly = Assembly(command)
            assemblies[key] = assembly
        try:
            assembly.add(command, packet.capdata)
        except ValueError as error:
            malformed += 1
            print(f"malformed_fragment\tframe={packet.frame}\t{error}")
            continue
        if command.more_fragments == 0 or assembly.complete:
            complete.append(assembly)
            assemblies.pop(key, None)

    video_counts = collections.Counter()
    jpeg_counts = collections.Counter()
    complete_videos = 0
    incomplete_jpegs = 0
    exported = 0
    rows: list[dict[str, object]] = []
    out_dir = args.export_jpegs
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)
    payload_dir = args.export_payloads
    if payload_dir:
        payload_dir.mkdir(parents=True, exist_ok=True)

    manifest_records: list[dict[str, object]] = []
    stem = safe_stem(args.pcap)

    for index, assembly in enumerate(complete, start=1):
        packet = UsbPacket(
            frame=assembly.last_command.frame,
            time=assembly.last_command.time,
            endpoint=args.bulk_out,
            data_len=len(assembly.data),
            capdata=bytes(assembly.data),
        )
        header = parse_video_header(packet, assembly.first_command)
        if header is None:
            if args.verbose:
                prefix = bytes(assembly.data[:16]).hex(" ")
                print(
                    "payload_unknown\t"
                    f"idx={index}\tcmd_frame={assembly.first_command.frame}\t"
                    f"dest=0x{assembly.first_command.dest:08x}\t"
                    f"total=0x{assembly.first_command.total_len:x}\t"
                    f"fragments={assembly.fragments}\tprefix={prefix}"
                )
            continue

        complete_videos += 1
        video_counts[(header.video_type, header.image_format)] += 1
        eoi = jpeg_eoi_offset(packet.capdata, header.jpeg_soi_offset)
        if header.jpeg_soi_offset >= 0:
            jpeg_counts[(header.video_type, header.jpeg_width, header.jpeg_height, eoi is not None)] += 1
        if header.jpeg_soi_offset >= 0 and eoi is None:
            incomplete_jpegs += 1

        if not args.summary_only:
            print(
                "video_reassembled\t"
                f"idx={index}\tcmd_frame={assembly.first_command.frame}\t"
                f"last_cmd_frame={assembly.last_command.frame}\t"
                f"time={assembly.last_command.time}\t"
                f"dest=0x{assembly.first_command.dest:08x}\t"
                f"total=0x{assembly.first_command.total_len:x}\t"
                f"fragments={assembly.fragments}\t"
                f"type=0x{header.video_type:x}\tformat=0x{header.image_format:x}\t"
                f"seq=0x{header.sequence:08x}\t"
                f"field={header.width_field}x{header.height_field}\t"
                f"jpeg={header.jpeg_width}x{header.jpeg_height}\t"
                f"soi={header.jpeg_soi_offset}\teoi={eoi}\t"
                f"components={header.jpeg_components}"
            )

        exported_name = ""
        payload_name = ""
        jpeg_name = ""
        salvaged = False
        payload = bytes(packet.capdata)
        if payload_dir and header.jpeg_soi_offset >= 0:
            prefix = (
                f"{stem}_idx{index:04d}_frame{assembly.last_command.frame}_"
                f"type{header.video_type:02x}_{header.jpeg_width}x{header.jpeg_height}"
            )
            payload_name = f"{prefix}.payload.bin"
            (payload_dir / payload_name).write_bytes(payload)

        jpeg = b""
        if (out_dir or payload_dir) and header.jpeg_soi_offset >= 0:
            if eoi is not None:
                jpeg = packet.capdata[header.jpeg_soi_offset:eoi]
            elif args.salvage_eoi:
                jpeg = trim_trailing_zeroes(packet.capdata[header.jpeg_soi_offset:]) + b"\xff\xd9"
                salvaged = True
            if jpeg:
                suffix = "_salvaged" if salvaged else ""
                name = (
                    f"{pathlib.Path(args.pcap).stem}_idx{index:04d}_"
                    f"frame{assembly.last_command.frame}_type{header.video_type:02x}_"
                    f"{header.jpeg_width}x{header.jpeg_height}{suffix}.jpg"
                )
                if out_dir:
                    (out_dir / name).write_bytes(jpeg)
                    exported_name = name
                    exported += 1
                if payload_dir:
                    jpeg_name = f"{prefix}{suffix}.jpg"
                    (payload_dir / jpeg_name).write_bytes(jpeg)

        if payload_dir and header.jpeg_soi_offset >= 0:
            metadata = {
                "index": index,
                "pcap": args.pcap,
                "command": {
                    "frame": assembly.first_command.frame,
                    "time": assembly.first_command.time,
                    "session": assembly.first_command.session,
                    "total_len": assembly.first_command.total_len,
                    "dest": assembly.first_command.dest,
                    "fragment_len": assembly.first_command.fragment_len,
                    "fragment_offset": assembly.first_command.fragment_offset,
                    "more_fragments": assembly.first_command.more_fragments,
                    "last_frame": assembly.last_command.frame,
                    "fragments": assembly.fragments,
                },
                "video": {
                    "frame": assembly.last_command.frame,
                    "time": assembly.last_command.time,
                    "command_frame": assembly.first_command.frame,
                    "payload_len": len(payload),
                    "type": header.video_type,
                    "data_len": header.data_len,
                    "sequence": header.sequence,
                    "flags_or_format_hint": header.flags_or_format_hint,
                    "width_field": header.width_field,
                    "height_field": header.height_field,
                    "canvas_width": header.canvas_width,
                    "canvas_height": header.canvas_height,
                    "start_addr": header.start_addr,
                    "end_addr": header.end_addr,
                    "image_format": header.image_format,
                    "jpeg_soi_offset": header.jpeg_soi_offset,
                    "jpeg_width": header.jpeg_width,
                    "jpeg_height": header.jpeg_height,
                    "sof_marker": header.sof_marker,
                    "jpeg_precision": header.jpeg_precision,
                    "jpeg_components": header.jpeg_components,
                    "complete": eoi is not None,
                    "salvaged": salvaged,
                },
                "files": {
                    "payload": payload_name,
                    "jpeg": jpeg_name,
                },
            }
            meta_name = f"{prefix}.json"
            (payload_dir / meta_name).write_text(json.dumps(metadata, indent=2), encoding="utf-8")
            manifest_records.append(metadata)

        rows.append(
            {
                "idx": index,
                "cmd_frame": assembly.first_command.frame,
                "last_cmd_frame": assembly.last_command.frame,
                "time": assembly.last_command.time,
                "dest": f"0x{assembly.first_command.dest:08x}",
                "total": f"0x{assembly.first_command.total_len:x}",
                "fragments": assembly.fragments,
                "type": f"0x{header.video_type:x}",
                "format": f"0x{header.image_format:x}",
                "sequence": f"0x{header.sequence:08x}",
                "field": f"{header.width_field}x{header.height_field}",
                "jpeg": f"{header.jpeg_width}x{header.jpeg_height}",
                "start_addr": f"0x{header.start_addr:08x}",
                "end_addr": f"0x{header.end_addr:08x}",
                "span": f"0x{header.end_addr - header.start_addr:x}",
                "soi": header.jpeg_soi_offset,
                "eoi": eoi if eoi is not None else "",
                "complete": eoi is not None,
                "salvaged": salvaged,
                "components": header.jpeg_components or "",
                "file": exported_name,
            }
        )

    if args.report_csv:
        args.report_csv.parent.mkdir(parents=True, exist_ok=True)
        with args.report_csv.open("w", newline="", encoding="utf-8") as f:
            fieldnames = [
                "idx",
                "cmd_frame",
                "last_cmd_frame",
                "time",
                "dest",
                "total",
                "fragments",
                "type",
                "format",
                "sequence",
                "field",
                "jpeg",
                "start_addr",
                "end_addr",
                "span",
                "soi",
                "eoi",
                "complete",
                "salvaged",
                "components",
                "file",
            ]
            writer = csv.DictWriter(f, fieldnames=fieldnames)
            writer.writeheader()
            writer.writerows(rows)

    if args.report_html:
        args.report_html.parent.mkdir(parents=True, exist_ok=True)
        args.report_html.write_text(render_html_report(args.pcap, rows), encoding="utf-8")

    if payload_dir:
        manifest = {
            "pcap": args.pcap,
            "count": len(manifest_records),
            "records": manifest_records,
        }
        manifest_path = payload_dir / f"{stem}_manifest.json"
        manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        print(f"manifest\t{manifest_path}\tcount={len(manifest_records)}")

    print("# reassembly_summary")
    print(f"packets\t{len(packets)}")
    print(f"complete_transfers\t{len(complete)}")
    print(f"open_transfers\t{len(assemblies)}")
    print(f"malformed_fragments\t{malformed}")
    print(f"video_payloads\t{complete_videos}")
    print(f"incomplete_jpegs\t{incomplete_jpegs}")
    print(f"exported_jpegs\t{exported}")
    for (video_type, image_format), count in sorted(video_counts.items()):
        print(f"video_count\ttype=0x{video_type:x}\tformat=0x{image_format:x}\tcount={count}")
    for (video_type, width, height, has_eoi), count in sorted(jpeg_counts.items()):
        print(
            "jpeg_count\t"
            f"type=0x{video_type:x}\tjpeg={width}x{height}\t"
            f"complete={has_eoi}\tcount={count}"
        )
    return 0


def render_html_report(pcap: str, rows: list[dict[str, object]]) -> str:
    table_rows = []
    for row in rows:
        image = ""
        if row["file"]:
            image = f'<img src="{html.escape(str(row["file"]))}" loading="lazy">'
        cells = [
            image,
            row["idx"],
            row["time"],
            row["type"],
            row["sequence"],
            row["dest"],
            row["total"],
            row["fragments"],
            row["field"],
            row["jpeg"],
            row["start_addr"],
            row["end_addr"],
            row["span"],
            row["complete"],
            row["salvaged"],
            row["components"],
        ]
        table_rows.append(
            "<tr>"
            + "".join(f"<td>{cell}</td>" if isinstance(cell, str) and cell.startswith("<img") else f"<td>{html.escape(str(cell))}</td>" for cell in cells)
            + "</tr>"
        )
    return f"""<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>T6 reassembled video report</title>
<style>
body {{ font-family: system-ui, sans-serif; margin: 16px; background: #111; color: #eee; }}
table {{ border-collapse: collapse; width: 100%; font-size: 12px; }}
th, td {{ border: 1px solid #333; padding: 4px 6px; vertical-align: top; }}
th {{ position: sticky; top: 0; background: #222; }}
img {{ width: 240px; height: auto; display: block; }}
code {{ color: #9cf; }}
</style>
</head>
<body>
<h1>T6 reassembled video report</h1>
<p><code>{html.escape(pcap)}</code></p>
<table>
<thead><tr>
<th>image</th><th>idx</th><th>time</th><th>type</th><th>seq</th><th>dest</th><th>total</th><th>frags</th><th>field</th><th>jpeg</th><th>start</th><th>end</th><th>span</th><th>complete</th><th>salvaged</th><th>components</th>
</tr></thead>
<tbody>
{''.join(table_rows)}
</tbody>
</table>
</body>
</html>
"""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pcap")
    parser.add_argument("--tshark")
    parser.add_argument("--bulk-out", type=lambda s: int(s, 0), default=DEFAULT_BULK_OUT)
    parser.add_argument("--summary-only", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--export-jpegs", type=pathlib.Path)
    parser.add_argument("--export-payloads", type=pathlib.Path)
    parser.add_argument("--salvage-eoi", action="store_true")
    parser.add_argument("--report-csv", type=pathlib.Path)
    parser.add_argument("--report-html", type=pathlib.Path)
    return summarize_reassembled(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
