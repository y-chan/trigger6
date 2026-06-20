#!/usr/bin/env python3
"""Reassemble fragmented Trigger6 display bulk transfers and inspect video payloads."""

from __future__ import annotations

import argparse
import collections
import pathlib

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
    out_dir = args.export_jpegs
    if out_dir:
        out_dir.mkdir(parents=True, exist_ok=True)

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

        if out_dir and header.jpeg_soi_offset >= 0 and eoi is not None:
            name = (
                f"{pathlib.Path(args.pcap).stem}_idx{index:04d}_"
                f"frame{assembly.last_command.frame}_type{header.video_type:02x}_"
                f"{header.jpeg_width}x{header.jpeg_height}.jpg"
            )
            (out_dir / name).write_bytes(packet.capdata[header.jpeg_soi_offset:eoi])
            exported += 1

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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pcap")
    parser.add_argument("--tshark")
    parser.add_argument("--bulk-out", type=lambda s: int(s, 0), default=DEFAULT_BULK_OUT)
    parser.add_argument("--summary-only", action="store_true")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--export-jpegs", type=pathlib.Path)
    return summarize_reassembled(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
