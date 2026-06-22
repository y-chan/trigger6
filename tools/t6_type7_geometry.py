#!/usr/bin/env python3
"""Estimate Trigger6 type7 JPEG placement from replay manifests."""

from __future__ import annotations

import argparse
import collections
import json
import pathlib


TYPE4_TO_TYPE7_START_DELTA = 0x3C000
TYPE4_TO_TYPE7_END_DELTA = 0x1E000


def parse_int(value: str) -> int:
    return int(value, 0)


def load_manifest(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def type4_zones(records: list[dict]) -> list[tuple[int, int]]:
    zones = collections.Counter()
    for record in records:
        video = record["video"]
        if video["type"] == 4:
            zones[(video["start_addr"], video["end_addr"])] += 1
    return [zone for zone, _count in zones.most_common()]


def previous_type4_zone(records: list[dict], index: int) -> tuple[int, int] | None:
    for record in reversed(records[:index]):
        video = record["video"]
        if video["type"] == 4:
            return video["start_addr"], video["end_addr"]
    return None


def matching_type4_zone(
    type7_start: int,
    type7_end: int,
    zones: list[tuple[int, int]],
) -> tuple[int, int] | None:
    expected = (
        type7_start - TYPE4_TO_TYPE7_START_DELTA,
        type7_end - TYPE4_TO_TYPE7_END_DELTA,
    )
    if expected in zones:
        return expected

    containing = [
        zone
        for zone in zones
        if zone[0] <= type7_start <= zone[1] and zone[0] <= type7_end <= zone[1]
    ]
    if containing:
        return containing[0]
    return None


def best_relative_zone(
    type7_start: int,
    type7_end: int,
    zones: list[tuple[int, int]],
) -> tuple[int, int] | None:
    if not zones:
        return None
    return min(zones, key=lambda zone: abs(type7_start - zone[0]) + abs(type7_end - zone[1]))


def format_offset(offset: int, pitch: int) -> str:
    if offset < 0:
        return "-"
    return f"{offset % pitch},{offset // pitch}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=pathlib.Path)
    parser.add_argument("--pitch", type=parse_int, default=1920)
    parser.add_argument(
        "--extra-pitch",
        action="append",
        type=parse_int,
        default=[],
        help="Additional pitch candidate, e.g. 0x780 or 2048",
    )
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    records = manifest["records"]
    zones = type4_zones(records)
    pitches = [args.pitch, *args.extra_pitch]

    print(f"manifest\t{args.manifest}")
    print(f"records\t{len(records)}")
    print("# type4_zones")
    for index, (start, end) in enumerate(zones, start=1):
        print(f"zone\tindex={index}\tstart=0x{start:08x}\tend=0x{end:08x}\tspan=0x{end-start:x}")

    print("# type7_geometry")
    header = [
        "idx",
        "seq",
        "jpeg",
        "t7_start",
        "t7_end",
        "zone_start",
        "zone_end",
        "offset_start",
        "offset_end",
        "prev_type4_start",
        "prev_type4_end",
        "delta_prev_start",
        "delta_prev_end",
    ]
    for pitch in pitches:
        header.append(f"xy_start_p{pitch}")
        header.append(f"xy_end_p{pitch}")
    print("\t".join(header))

    delta_counts = collections.Counter()
    jpeg_counts = collections.Counter()

    for record_index, record in enumerate(records):
        video = record["video"]
        if video["type"] != 7:
            continue
        jpeg = f"{video['jpeg_width']}x{video['jpeg_height']}"
        start = video["start_addr"]
        end = video["end_addr"]
        zone = matching_type4_zone(start, end, zones)
        if zone is None:
            zone = best_relative_zone(start, end, zones)
        if zone is None:
            zone_start = zone_end = None
            offset_start = offset_end = None
        else:
            zone_start, zone_end = zone
            offset_start = start - zone_start
            offset_end = end - zone_start
        prev_zone = previous_type4_zone(records, record_index)
        if prev_zone is None:
            prev_start = prev_end = None
            delta_prev_start = delta_prev_end = None
        else:
            prev_start, prev_end = prev_zone
            delta_prev_start = start - prev_start
            delta_prev_end = end - prev_end
            delta_counts[(jpeg, delta_prev_start, delta_prev_end, end - start)] += 1
        jpeg_counts[(jpeg, end - start)] += 1

        row = [
            str(record["index"]),
            f"0x{video['sequence']:08x}",
            jpeg,
            f"0x{start:08x}",
            f"0x{end:08x}",
            "-" if zone_start is None else f"0x{zone_start:08x}",
            "-" if zone_end is None else f"0x{zone_end:08x}",
            "-" if offset_start is None else f"0x{offset_start:x}",
            "-" if offset_end is None else f"0x{offset_end:x}",
            "-" if prev_start is None else f"0x{prev_start:08x}",
            "-" if prev_end is None else f"0x{prev_end:08x}",
            "-" if delta_prev_start is None else f"0x{delta_prev_start:x}",
            "-" if delta_prev_end is None else f"0x{delta_prev_end:x}",
        ]
        for pitch in pitches:
            row.append("-" if offset_start is None else format_offset(offset_start, pitch))
            row.append("-" if offset_end is None else format_offset(offset_end, pitch))
        print("\t".join(row))

    print("# type7_jpeg_span_summary")
    for (jpeg, span), count in jpeg_counts.most_common():
        print(f"jpeg_span\tcount={count}\tjpeg={jpeg}\tspan=0x{span:x}")

    print("# type7_delta_from_previous_type4_summary")
    for (jpeg, delta_start, delta_end, span), count in delta_counts.most_common():
        print(
            "delta_prev"
            f"\tcount={count}"
            f"\tjpeg={jpeg}"
            f"\tdelta_start=0x{delta_start:x}"
            f"\tdelta_end=0x{delta_end:x}"
            f"\tspan=0x{span:x}"
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
