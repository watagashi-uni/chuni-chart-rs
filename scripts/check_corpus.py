#!/usr/bin/env python3
"""Check local C2S files and optionally render every mode in bounded workers.

No chart or image is added to the repository. A manifest contains a JSON array
of paths, or an object with a `files` array. Results are JSONL plus a summary;
successful PNGs are validated and discarded. Linux/prlimit, stdlib only.
"""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import signal
import shutil
import struct
import subprocess
import tempfile
import time
import zlib


def run(binary, args, *, payload=None, output=subprocess.PIPE):
    command = ["prlimit", "--as=268435456", "--cpu=12", "--core=0", "--",
               "nice", "-n", "10", str(binary), *args]
    process = subprocess.Popen(command, stdin=subprocess.PIPE if payload is not None else subprocess.DEVNULL,
                               stdout=output, stderr=subprocess.PIPE, start_new_session=True)
    try:
        stdout, stderr = process.communicate(payload, timeout=15)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise RuntimeError("worker wall timeout")
    if process.returncode:
        raise RuntimeError(f"worker exit {process.returncode}: " + stderr.decode(errors="replace")[:2000].strip())
    return stdout


def png_info(file):
    size = file.tell()
    if not 32 <= size <= 16 << 20:
        raise ValueError("PNG output byte limit")
    file.seek(0)
    digest = hashlib.sha256()

    def read(n):
        data = file.read(n)
        if len(data) != n:
            raise ValueError("truncated PNG")
        digest.update(data)
        return data

    if read(8) != b"\x89PNG\r\n\x1a\n":
        raise ValueError("invalid PNG signature")
    dimensions = None
    idat = False
    while file.tell() < size:
        length, kind = struct.unpack(">I4s", read(8))
        if length > size - file.tell() - 4:
            raise ValueError("invalid PNG chunk length")
        body = read(length)
        crc = struct.unpack(">I", read(4))[0]
        if zlib.crc32(kind + body) & 0xffffffff != crc:
            raise ValueError("invalid PNG checksum")
        if kind == b"IHDR":
            w, h, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", body)
            if dimensions is not None or not (0 < w <= 32768 and 0 < h <= 32768 and w * h <= 40_000_000):
                raise ValueError("invalid PNG dimensions")
            if (depth, color, compression, filtering, interlace) != (8, 6, 0, 0, 0):
                raise ValueError("unexpected PNG encoding")
            dimensions = (w, h)
        elif kind == b"IDAT":
            idat = True
        elif kind == b"IEND":
            if length or not idat or dimensions is None or file.tell() != size:
                raise ValueError("invalid PNG end")
            return {"width": dimensions[0], "height": dimensions[1], "bytes": size, "sha256": digest.hexdigest()}
    raise ValueError("missing PNG end")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/chuni-chart-rs"))
    parser.add_argument("--check-binary", type=Path, default=Path("target/release/examples/check_corpus"))
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--render", action="store_true")
    parser.add_argument("--modes", nargs="+", choices=["preview", "judge", "fastattack"], default=["preview", "judge", "fastattack"])
    args = parser.parse_args()
    if not 1 <= args.jobs <= 32:
        parser.error("jobs must be 1..32")
    args.binary, args.check_binary = args.binary.resolve(), args.check_binary.resolve()
    source = json.loads(args.manifest.read_text())
    paths = sorted(set(source["files"] if isinstance(source, dict) else source))
    args.output.mkdir(parents=True, exist_ok=True)
    if (args.output / "checks.jsonl").exists():
        parser.error("output already contains a corpus run; use a new directory")
    binary_source = str(args.binary)
    # Integration tests can rebuild target/release while this long scan runs.
    # Execute immutable copies so every row belongs to the recorded build.
    binaries = args.output.resolve() / "bin"
    binaries.mkdir()
    for attribute, name in [("binary", "chuni-chart-rs"), ("check_binary", "check_corpus")]:
        target = binaries / name
        shutil.copyfile(getattr(args, attribute), target)
        target.chmod(0o700)
        setattr(args, attribute, target)
    version = {"binary": binary_source, "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               "checker_sha256": hashlib.sha256(args.check_binary.read_bytes()).hexdigest(),
               "files": len(paths), "jobs": args.jobs, "memory_bytes": 256 << 20, "cpu_seconds": 12,
               "wall_seconds": 15, "modes": args.modes if args.render else []}
    (args.output / "run.json").write_text(json.dumps(version, indent=2))
    started = time.monotonic()

    def check(path):
        begin = time.monotonic()
        row = {"path": path}
        try:
            row["input_sha256"] = hashlib.sha256(Path(path).read_bytes()).hexdigest()
            row.update(json.loads(run(args.check_binary, [path])))
        except Exception as error:
            row.update(ok=False, error=str(error))
        row["seconds"] = round(time.monotonic() - begin, 4)
        return row

    checks = []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool, (args.output / "checks.jsonl").open("w") as log:
        jobs = [pool.submit(check, path) for path in paths]
        for future in as_completed(jobs):
            row = future.result()
            checks.append(row)
            log.write(json.dumps(row) + "\n")
            if len(checks) % 500 == 0 or len(checks) == len(paths):
                log.flush()
                print(f"check {len(checks)}/{len(paths)}; failed {sum(not c['ok'] for c in checks)}", flush=True)
    errors = Counter(c.get("error", "") for c in checks if not c["ok"])
    summary = {**version, "parsed": sum(c["ok"] for c in checks), "parse_errors": dict(errors), "renders": {}}

    def render(row, mode):
        begin = time.monotonic()
        result = {"path": row["path"], "input_sha256": row["input_sha256"], "mode": mode}
        try:
            raw = Path(row["path"]).read_bytes()
            if hashlib.sha256(raw).hexdigest() != row["input_sha256"]:
                raise ValueError("input changed during corpus run")
            payload = json.dumps({"chart": raw.decode("utf-8-sig"), "options": {
                "judge": mode != "preview", "early_attack_only": mode == "fastattack", "divisions": True,
                "easy": False, "format": "png", "zoom": 1.0, "column": None}}).encode()
            with tempfile.TemporaryFile() as image:
                run(args.binary, ["_worker"], payload=payload, output=image)
                result.update(png_info(image), ok=True)
        except Exception as error:
            result.update(ok=False, error=str(error))
        result["seconds"] = round(time.monotonic() - begin, 4)
        return result

    if args.render:
        counts = {mode: Counter() for mode in args.modes}
        eligible = [c for c in checks if c["ok"]]
        total, done, last = len(eligible) * len(args.modes), 0, time.monotonic()
        with ThreadPoolExecutor(max_workers=args.jobs) as pool, (args.output / "renders.jsonl").open("w") as log:
            jobs = [pool.submit(render, row, mode) for row in eligible for mode in args.modes]
            for future in as_completed(jobs):
                row = future.result()
                counts[row["mode"]]["passed" if row["ok"] else row["error"]] += 1
                log.write(json.dumps(row) + "\n")
                done += 1
                if time.monotonic() - last >= 30 or done == total:
                    last = time.monotonic()
                    log.flush()
                    print(f"render {done}/{total}; failures {sum(sum(v for k,v in c.items() if k != 'passed') for c in counts.values())}; elapsed {round(last-started)}s", flush=True)
        summary["renders"] = {mode: dict(count) for mode, count in counts.items()}
    summary["seconds"] = round(time.monotonic() - started, 2)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2), flush=True)
    return int(any(not c["ok"] for c in checks) or any(any(k != "passed" for k in c) for c in summary["renders"].values()))


if __name__ == "__main__":
    raise SystemExit(main())
