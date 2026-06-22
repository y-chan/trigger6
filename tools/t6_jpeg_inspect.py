#!/usr/bin/env python3
"""Print JPEG marker, sampling, comment, and quantization table details."""

from __future__ import annotations

import argparse
import pathlib


SOF_MARKERS = {
    0xC0: "SOF0 baseline",
    0xC1: "SOF1 extended sequential",
    0xC2: "SOF2 progressive",
}


def parse_jpeg(data: bytes) -> dict:
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        raise ValueError("not a JPEG file")

    info = {
        "comments": [],
        "quant_tables": {},
        "huffman_tables": [],
        "sof": None,
        "adobe": None,
        "jfif": None,
    }

    i = 2
    while i < len(data):
        while i < len(data) and data[i] == 0xFF:
            i += 1
        if i >= len(data):
            break
        marker = data[i]
        i += 1
        if marker == 0xD9:
            break
        if marker == 0xDA:
            break
        if marker in {0x01} or 0xD0 <= marker <= 0xD7:
            continue
        if i + 2 > len(data):
            break
        seg_len = int.from_bytes(data[i : i + 2], "big")
        i += 2
        if seg_len < 2 or i + seg_len - 2 > len(data):
            break
        payload = data[i : i + seg_len - 2]
        i += seg_len - 2

        if marker == 0xFE:
            info["comments"].append(payload.decode("latin-1", errors="replace"))
        elif marker == 0xDB:
            parse_dqt(payload, info["quant_tables"])
        elif marker == 0xC4:
            parse_dht(payload, info["huffman_tables"])
        elif marker in SOF_MARKERS:
            info["sof"] = parse_sof(marker, payload)
        elif marker == 0xE0 and payload.startswith(b"JFIF\0"):
            info["jfif"] = payload[:14].hex(" ")
        elif marker == 0xEE and payload.startswith(b"Adobe"):
            info["adobe"] = payload.hex(" ")

    return info


def parse_dqt(payload: bytes, tables: dict[int, list[int]]) -> None:
    i = 0
    while i < len(payload):
        spec = payload[i]
        i += 1
        precision = spec >> 4
        table_id = spec & 0x0F
        count = 64 * (2 if precision else 1)
        raw = payload[i : i + count]
        i += count
        if precision:
            values = [int.from_bytes(raw[j : j + 2], "big") for j in range(0, len(raw), 2)]
        else:
            values = list(raw)
        tables[table_id] = values


def parse_dht(payload: bytes, tables: list[tuple[int, int, int]]) -> None:
    i = 0
    while i + 17 <= len(payload):
        spec = payload[i]
        i += 1
        table_class = spec >> 4
        table_id = spec & 0x0F
        counts = payload[i : i + 16]
        i += 16
        symbol_count = sum(counts)
        i += symbol_count
        tables.append((table_class, table_id, symbol_count))


def parse_sof(marker: int, payload: bytes) -> dict:
    if len(payload) < 6:
        return {"marker": marker}
    precision = payload[0]
    height = int.from_bytes(payload[1:3], "big")
    width = int.from_bytes(payload[3:5], "big")
    components = payload[5]
    parsed = []
    offset = 6
    for _ in range(components):
        if offset + 3 > len(payload):
            break
        component_id = payload[offset]
        sampling = payload[offset + 1]
        qtable = payload[offset + 2]
        parsed.append(
            {
                "id": component_id,
                "h": sampling >> 4,
                "v": sampling & 0x0F,
                "q": qtable,
            }
        )
        offset += 3
    return {
        "marker": marker,
        "precision": precision,
        "width": width,
        "height": height,
        "components": parsed,
    }


def print_info(path: pathlib.Path) -> None:
    info = parse_jpeg(path.read_bytes())
    print(f"file\t{path}")
    sof = info["sof"]
    if sof:
        marker = sof["marker"]
        components = ",".join(
            f"id{c['id']}:{c['h']}x{c['v']}:q{c['q']}" for c in sof["components"]
        )
        print(
            "sof"
            f"\tmarker=0x{marker:02x}"
            f"\tname={SOF_MARKERS.get(marker, '?')}"
            f"\tsize={sof['width']}x{sof['height']}"
            f"\tprecision={sof['precision']}"
            f"\tcomponents={components}"
        )
    if info["jfif"]:
        print(f"jfif\t{info['jfif']}")
    if info["adobe"]:
        print(f"adobe\t{info['adobe']}")
    for comment in info["comments"]:
        print(f"comment\t{comment}")
    for table_id, values in sorted(info["quant_tables"].items()):
        print(
            f"dqt\tid={table_id}"
            f"\tmin={min(values)}"
            f"\tmax={max(values)}"
            f"\tsum={sum(values)}"
            f"\tvalues={','.join(str(v) for v in values)}"
        )
    if info["huffman_tables"]:
        summary = ",".join(f"class{cls}/id{tid}/n{count}" for cls, tid, count in info["huffman_tables"])
        print(f"dht\t{summary}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("jpeg", nargs="+", type=pathlib.Path)
    args = parser.parse_args()
    for index, path in enumerate(args.jpeg):
        if index:
            print()
        print_info(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
