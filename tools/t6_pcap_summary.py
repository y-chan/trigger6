#!/usr/bin/env python3
"""Summarize MCT Trigger6 USB traffic from a pcapng file.

This is intentionally small and tshark-backed so the same script can be used for
Linux usbmon captures and Windows USBPcap captures as long as Wireshark exposes
the usual usb.* fields.
"""

from __future__ import annotations

import argparse
import collections
import shutil
import struct
import subprocess
import sys
from dataclasses import dataclass


DEFAULT_BULK_OUT = 0x02
DEFAULT_INTERRUPT_IN = 0x83
VIDEO_HEADER_LEN = 0x30
KNOWN_VIDEO_TYPES = {0x03, 0x04, 0x07}
KNOWN_IMAGE_FORMATS = {0x00, 0x06, 0x09, 0x0D}


@dataclass
class UsbPacket:
    frame: int
    time: str
    endpoint: int | None
    data_len: int
    capdata: bytes


@dataclass
class BulkCommand:
    frame: int
    time: str
    session: int
    total_len: int
    dest: int
    fragment_len: int
    fragment_offset: int
    more_fragments: int


@dataclass
class VideoHeader:
    frame: int
    time: str
    session: int
    command_frame: int
    payload_len: int
    video_type: int
    data_len: int
    sequence: int
    flags_or_format_hint: int
    width_field: int
    height_field: int
    canvas_width: int | None
    canvas_height: int | None
    start_addr: int
    end_addr: int
    image_format: int
    jpeg_soi_offset: int
    jpeg_width: int | None
    jpeg_height: int | None
    sof_marker: int | None
    jpeg_precision: int | None
    jpeg_components: str | None


def parse_jpeg_size(data: bytes, soi_offset: int) -> tuple[int, int] | tuple[None, None]:
    if soi_offset < 0 or soi_offset + 4 >= len(data):
        return None, None
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
        seg_len = struct.unpack_from(">H", data, pos)[0]
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
            if seg_len >= 7:
                height = struct.unpack_from(">H", data, pos + 3)[0]
                width = struct.unpack_from(">H", data, pos + 5)[0]
                return width, height
        pos += seg_len
    return None, None


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
        seg_len = struct.unpack_from(">H", data, pos)[0]
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
                comps.append(f"id{comp_id}:{sampling >> 4}x{sampling & 0x0f}:q{qtable}")
                comp_pos += 3
            return marker, precision, ",".join(comps)
        pos += seg_len
    return None, None, None


def parse_int(value: str) -> int | None:
    if not value:
        return None
    return int(value, 0)


def parse_capdata(value: str) -> bytes:
    if not value:
        return b""
    value = value.replace(":", "").replace(" ", "")
    return bytes.fromhex(value)


def tshark_path(explicit: str | None) -> str:
    if explicit:
        return explicit
    for candidate in (
        "tshark",
        "/Applications/Wireshark.app/Contents/MacOS/tshark",
    ):
        path = shutil.which(candidate) if "/" not in candidate else candidate
        if path and shutil.which(path):
            return path
        if "/" in candidate and shutil.which(candidate) is None:
            try:
                subprocess.run(
                    [candidate, "-v"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    check=False,
                )
                return candidate
            except OSError:
                pass
    raise SystemExit("tshark not found; install Wireshark or pass --tshark")


def load_packets(pcap: str, tshark: str) -> list[UsbPacket]:
    cmd = [
        tshark,
        "-r",
        pcap,
        "-T",
        "fields",
        "-E",
        "separator=\t",
        "-e",
        "frame.number",
        "-e",
        "frame.time_relative",
        "-e",
        "usb.endpoint_address",
        "-e",
        "usb.data_len",
        "-e",
        "usb.capdata",
    ]
    proc = subprocess.run(cmd, text=True, capture_output=True, check=False)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)

    packets: list[UsbPacket] = []
    for line in proc.stdout.splitlines():
        fields = line.split("\t")
        fields += [""] * (5 - len(fields))
        frame_s, time_s, ep_s, data_len_s, capdata_s = fields[:5]
        if not frame_s:
            continue
        data_len = parse_int(data_len_s) or 0
        packets.append(
            UsbPacket(
                frame=int(frame_s),
                time=time_s,
                endpoint=parse_int(ep_s),
                data_len=data_len,
                capdata=parse_capdata(capdata_s),
            )
        )
    return packets


def parse_bulk_command(packet: UsbPacket) -> BulkCommand | None:
    if packet.data_len != 32 or len(packet.capdata) < 32:
        return None
    session, total_len, dest, fragment_len, fragment_offset = struct.unpack_from(
        "<IIIII", packet.capdata, 0
    )
    more_fragments = packet.capdata[0x14]
    return BulkCommand(
        frame=packet.frame,
        time=packet.time,
        session=session,
        total_len=total_len,
        dest=dest,
        fragment_len=fragment_len,
        fragment_offset=fragment_offset,
        more_fragments=more_fragments,
    )


def parse_video_header(packet: UsbPacket, command: BulkCommand) -> VideoHeader | None:
    if len(packet.capdata) < VIDEO_HEADER_LEN:
        return None
    video_type, data_len, sequence, flags_or_format_hint = struct.unpack_from(
        "<IIII", packet.capdata, 0
    )
    width_field, height_field = struct.unpack_from("<HH", packet.capdata, 0x10)
    if video_type not in KNOWN_VIDEO_TYPES:
        return None
    if video_type == 0x07:
        canvas_packed, start_addr, end_addr, _unk9, image_format = struct.unpack_from(
            "<IIIII", packet.capdata, 0x14
        )
        canvas_width = canvas_packed & 0xFFFF
        canvas_height = canvas_packed >> 16
    else:
        start_addr, end_addr, _unk9, image_format = struct.unpack_from(
            "<IIII", packet.capdata, 0x14
        )
        canvas_width = None
        canvas_height = None
    if image_format not in KNOWN_IMAGE_FORMATS:
        return None
    if data_len > packet.data_len:
        return None
    if video_type == 0x04 and image_format == 0x0D:
        jpeg_soi_offset = packet.capdata.find(b"\xff\xd8")
        if jpeg_soi_offset < 0:
            return None
    else:
        jpeg_soi_offset = packet.capdata.find(b"\xff\xd8")
    jpeg_width, jpeg_height = parse_jpeg_size(packet.capdata, jpeg_soi_offset)
    sof_marker, jpeg_precision, jpeg_components = parse_jpeg_details(
        packet.capdata, jpeg_soi_offset
    )
    return VideoHeader(
        frame=packet.frame,
        time=packet.time,
        session=command.session,
        command_frame=command.frame,
        payload_len=packet.data_len,
        video_type=video_type,
        data_len=data_len,
        sequence=sequence,
        flags_or_format_hint=flags_or_format_hint,
        width_field=width_field,
        height_field=height_field,
        canvas_width=canvas_width,
        canvas_height=canvas_height,
        start_addr=start_addr,
        end_addr=end_addr,
        image_format=image_format,
        jpeg_soi_offset=jpeg_soi_offset,
        jpeg_width=jpeg_width,
        jpeg_height=jpeg_height,
        sof_marker=sof_marker,
        jpeg_precision=jpeg_precision,
        jpeg_components=jpeg_components,
    )


def print_command(command: BulkCommand) -> None:
    print(
        "command\t"
        f"{command.frame}\t{command.time}\t"
        f"session={command.session}\t"
        f"total=0x{command.total_len:x}\t"
        f"dest=0x{command.dest:08x}\t"
        f"frag_len=0x{command.fragment_len:x}\t"
        f"frag_off=0x{command.fragment_offset:x}\t"
        f"more={command.more_fragments}"
    )


def print_video(header: VideoHeader) -> None:
    sof = f"sof=0x{header.sof_marker:x}" if header.sof_marker is not None else "sof=?"
    print(
        "video\t"
        f"{header.frame}\t{header.time}\t"
        f"session={header.session}\t"
        f"cmd_frame={header.command_frame}\t"
        f"payload=0x{header.payload_len:x}\t"
        f"type=0x{header.video_type:x}\t"
        f"data_len=0x{header.data_len:x}\t"
        f"seq={header.sequence}\t"
        f"hint=0x{header.flags_or_format_hint:x}\t"
        f"width_field=0x{header.width_field:x}\t"
        f"height_field=0x{header.height_field:x}\t"
        f"canvas={header.canvas_width}x{header.canvas_height}\t"
        f"start=0x{header.start_addr:x}\t"
        f"end=0x{header.end_addr:x}\t"
        f"format=0x{header.image_format:x}\t"
        f"jpeg_soi={header.jpeg_soi_offset}\t"
        f"jpeg={header.jpeg_width}x{header.jpeg_height}\t"
        f"{sof}\t"
        f"components={header.jpeg_components}"
    )


def print_jpeg_summary(headers: list[VideoHeader]) -> None:
    counts = collections.Counter()
    sizes = collections.Counter()
    for header in headers:
        if header.image_format != 0x0D or header.sof_marker is None:
            continue
        key = (header.video_type, header.sof_marker, header.jpeg_components or "?")
        counts[key] += 1
        sizes[(header.video_type, header.jpeg_width, header.jpeg_height, header.jpeg_components or "?")] += 1

    print("# jpeg_summary")
    for (video_type, sof_marker, components), count in sorted(counts.items()):
        name = {0xC0: "baseline", 0xC1: "extended-sequential", 0xC2: "progressive"}.get(
            sof_marker, "sof"
        )
        print(
            "jpeg_count\t"
            f"type=0x{video_type:x}\t"
            f"count={count}\t"
            f"sof=0x{sof_marker:02x}\t"
            f"name={name}\t"
            f"components={components}"
        )
    for (video_type, width, height, components), count in sorted(
        sizes.items(), key=lambda item: (-item[1], item[0])
    )[:80]:
        print(
            "jpeg_size\t"
            f"type=0x{video_type:x}\t"
            f"count={count}\t"
            f"jpeg={width}x{height}\t"
            f"components={components}"
        )


def print_interrupt(packet: UsbPacket) -> tuple[int, int, int]:
    flags = packet.capdata[0] if packet.capdata else 0
    value = struct.unpack_from("<I", packet.capdata, 0x0C)[0]
    event = packet.capdata[0x13]
    print(
        "interrupt\t"
        f"{packet.frame}\t{packet.time}\t"
        f"flags=0x{flags:02x}\t"
        f"value=0x{value:08x}\t"
        f"event=0x{event:02x}"
    )
    return flags, value, event


def summarize(args: argparse.Namespace) -> int:
    packets = load_packets(args.pcap, tshark_path(args.tshark))
    command_counts = collections.Counter()
    video_counts = collections.Counter()
    interrupt_counts = collections.Counter()
    pending: BulkCommand | None = None
    commands = videos = interrupts = 0
    video_headers: list[VideoHeader] = []
    printed = 0

    for packet in packets:
        if packet.endpoint == args.bulk_out and packet.capdata:
            command = parse_bulk_command(packet)
            if command is not None:
                commands += 1
                command_counts[(command.session, command.dest)] += 1
                pending = command
                if not args.summary_only and not args.video_only and (
                    args.limit is None or printed < args.limit
                ):
                    print_command(command)
                    printed += 1
                continue

            if pending:
                header = parse_video_header(packet, pending)
                if header is not None:
                    videos += 1
                    video_headers.append(header)
                    video_counts[(header.video_type, header.image_format)] += 1
                    if not args.summary_only and (
                        args.limit is None or printed < args.limit
                    ):
                        print_video(header)
                        printed += 1
                pending = None

        if packet.endpoint == args.interrupt_in and packet.data_len == 64 and packet.capdata:
            interrupts += 1
            should_print_interrupt = (
                not args.summary_only
                and not args.video_only
                and (args.limit is None or printed < args.limit)
            )
            flags, _value, event = print_interrupt(packet) if should_print_interrupt else (
                packet.capdata[0],
                struct.unpack_from("<I", packet.capdata, 0x0C)[0],
                packet.capdata[0x13],
            )
            if should_print_interrupt:
                printed += 1
            interrupt_counts[(flags, event)] += 1

    print("# summary")
    print(f"packets\t{len(packets)}")
    print(f"commands\t{commands}")
    print(f"video_payloads\t{videos}")
    print(f"interrupts\t{interrupts}")
    if not args.jpeg_summary_only:
        for (session, dest), count in sorted(command_counts.items()):
            print(f"command_count\tsession={session}\tdest=0x{dest:08x}\tcount={count}")
        for (video_type, image_format), count in sorted(video_counts.items()):
            print(
                f"video_count\ttype=0x{video_type:x}\t"
                f"format=0x{image_format:x}\tcount={count}"
            )
        for (flags, event), count in sorted(interrupt_counts.items()):
            print(f"interrupt_count\tflags=0x{flags:02x}\tevent=0x{event:02x}\tcount={count}")
    if args.jpeg_summary:
        print_jpeg_summary(video_headers)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("pcap", help="pcapng file to inspect")
    parser.add_argument("--tshark", help="path to tshark")
    parser.add_argument("--bulk-out", type=lambda s: int(s, 0), default=DEFAULT_BULK_OUT)
    parser.add_argument(
        "--interrupt-in", type=lambda s: int(s, 0), default=DEFAULT_INTERRUPT_IN
    )
    parser.add_argument("--summary-only", action="store_true")
    parser.add_argument("--video-only", action="store_true")
    parser.add_argument("--jpeg-summary", action="store_true")
    parser.add_argument(
        "--jpeg-summary-only",
        action="store_true",
        help="omit verbose count tables when printing --jpeg-summary",
    )
    parser.add_argument("--limit", type=int)
    return summarize(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
