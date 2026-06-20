#!/usr/bin/env python3
"""Extract Trigger6 JPEG video payloads from a USB pcap for replay experiments."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

TOOLS = pathlib.Path(__file__).resolve().parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import t6_pcap_summary as pcap_summary  # noqa: E402


def safe_stem(path: pathlib.Path) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", path.stem)


def hex_or_int(value: str) -> int:
    return int(value, 0)


def should_keep(
    header: pcap_summary.VideoHeader,
    args: argparse.Namespace,
) -> bool:
    if header.image_format != 0x0D:
        return False
    if header.jpeg_soi_offset < 0:
        return False
    if args.video_type is not None and header.video_type != args.video_type:
        return False
    if args.jpeg_width is not None and header.jpeg_width != args.jpeg_width:
        return False
    if args.jpeg_height is not None and header.jpeg_height != args.jpeg_height:
        return False
    if args.min_jpeg_width is not None and (
        header.jpeg_width is None or header.jpeg_width < args.min_jpeg_width
    ):
        return False
    if args.min_jpeg_height is not None and (
        header.jpeg_height is None or header.jpeg_height < args.min_jpeg_height
    ):
        return False
    return True


def collect_records(args: argparse.Namespace) -> list[tuple[pcap_summary.BulkCommand, pcap_summary.VideoHeader, bytes]]:
    packets = pcap_summary.load_packets(args.pcap, pcap_summary.tshark_path(args.tshark))
    pending: pcap_summary.BulkCommand | None = None
    records: list[tuple[pcap_summary.BulkCommand, pcap_summary.VideoHeader, bytes]] = []

    for packet in packets:
        if packet.endpoint != args.bulk_out or not packet.capdata:
            continue

        command = pcap_summary.parse_bulk_command(packet)
        if command is not None:
            pending = command
            continue

        if pending is None:
            continue

        header = pcap_summary.parse_video_header(packet, pending)
        if header is not None and should_keep(header, args):
            records.append((pending, header, packet.capdata))
            if args.limit is not None and len(records) >= args.limit:
                break
        pending = None

    return records


def record_metadata(
    index: int,
    pcap: pathlib.Path,
    command: pcap_summary.BulkCommand,
    header: pcap_summary.VideoHeader,
    payload_file: str,
    jpeg_file: str,
) -> dict[str, object]:
    return {
        "index": index,
        "pcap": str(pcap),
        "command": {
            "frame": command.frame,
            "time": command.time,
            "session": command.session,
            "total_len": command.total_len,
            "dest": command.dest,
            "fragment_len": command.fragment_len,
            "fragment_offset": command.fragment_offset,
            "more_fragments": command.more_fragments,
        },
        "video": {
            "frame": header.frame,
            "time": header.time,
            "command_frame": header.command_frame,
            "payload_len": header.payload_len,
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
        },
        "files": {
            "payload": payload_file,
            "jpeg": jpeg_file,
        },
    }


def slice_jpeg(payload: bytes, soi_offset: int) -> bytes:
    jpeg = payload[soi_offset:]
    eoi = jpeg.find(b"\xff\xd9")
    if eoi >= 0:
        return jpeg[: eoi + 2]
    return jpeg


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pcap", help="pcap/pcapng file to inspect")
    parser.add_argument("--out-dir", type=pathlib.Path, required=True)
    parser.add_argument("--tshark", help="path to tshark")
    parser.add_argument("--bulk-out", type=hex_or_int, default=pcap_summary.DEFAULT_BULK_OUT)
    parser.add_argument("--video-type", type=hex_or_int)
    parser.add_argument("--jpeg-width", type=int)
    parser.add_argument("--jpeg-height", type=int)
    parser.add_argument("--min-jpeg-width", type=int)
    parser.add_argument("--min-jpeg-height", type=int)
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()

    pcap_path = pathlib.Path(args.pcap)
    records = collect_records(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)

    stem = safe_stem(pcap_path)
    manifest: dict[str, object] = {
        "pcap": str(pcap_path),
        "count": len(records),
        "records": [],
    }

    manifest_records: list[dict[str, object]] = []
    for index, (command, header, payload) in enumerate(records, start=1):
        prefix = (
            f"{stem}_idx{index:03d}_frame{header.frame}_"
            f"type{header.video_type:02x}_{header.jpeg_width}x{header.jpeg_height}"
        )
        payload_path = args.out_dir / f"{prefix}.payload.bin"
        jpeg_path = args.out_dir / f"{prefix}.jpg"
        meta_path = args.out_dir / f"{prefix}.json"

        jpeg = slice_jpeg(payload, header.jpeg_soi_offset)
        payload_path.write_bytes(payload)
        jpeg_path.write_bytes(jpeg)

        metadata = record_metadata(
            index,
            pcap_path,
            command,
            header,
            payload_path.name,
            jpeg_path.name,
        )
        meta_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        manifest_records.append(metadata)

        print(
            "extracted\t"
            f"index={index}\t"
            f"frame={header.frame}\t"
            f"type=0x{header.video_type:x}\t"
            f"cmd_dest=0x{command.dest:08x}\t"
            f"payload={len(payload)}\t"
            f"jpeg={len(jpeg)}\t"
            f"size={header.jpeg_width}x{header.jpeg_height}\t"
            f"components={header.jpeg_components}\t"
            f"file={payload_path}"
        )

    manifest["records"] = manifest_records
    manifest_path = args.out_dir / f"{stem}_manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"manifest\t{manifest_path}\tcount={len(records)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
