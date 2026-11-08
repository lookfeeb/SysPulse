"""Read-only helper protocol probe with optional bounded CPU/disk load.

Only snapshot/shutdown requests are sent; no fan writes or real cache cleanup.
Outputs measurements under target/hw-validation. Run explicitly, not in CI.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import multiprocessing as mp
import os
from pathlib import Path
import queue
import subprocess
import threading
import time
import uuid


class ProcessMemory(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ("PeakWorkingSetSize", "WorkingSetSize",
        "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
        "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]


def process_resources(child):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    times = [wintypes.FILETIME() for _ in range(4)]
    handle = wintypes.HANDLE(int(child._handle))
    if not kernel.GetProcessTimes(handle, *(ctypes.byref(t) for t in times)):
        raise ctypes.WinError(ctypes.get_last_error())
    memory = ProcessMemory(cb=ctypes.sizeof(ProcessMemory))
    if not ctypes.WinDLL("psapi", use_last_error=True).GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    cpu = sum((t.dwHighDateTime << 32) | t.dwLowDateTime for t in times[2:]) / 10_000_000
    return {"cpuSeconds": cpu, "workingSetBytes": memory.WorkingSetSize, "privateBytes": memory.PrivateUsage}


def percentiles(values):
    values = sorted(values)
    return {name: round(values[int((len(values) - 1) * fraction)], 4)
            for name, fraction in (("p50", .5), ("p95", .95), ("p99", .99), ("max", 1))} if values else {}


def burn(until):
    value = 1
    while time.monotonic() < until:
        value = (value * 1664525 + 1013904223) & 0xffffffff


def disk_load(path, until):
    data = b"x" * (8 * 1024 * 1024)
    try:
        with path.open("xb+") as output:
            for _ in range(128):  # At most 1 GiB written; only 8 MiB retained.
                if time.monotonic() >= until:
                    break
                output.seek(0)
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
                output.seek(0)
                output.read(len(data))
                time.sleep(0.05)
    finally:
        path.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("helper", type=Path)
    parser.add_argument("--stress", action="store_true")
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--burst", type=int, default=0, help="Extra sequential cached snapshot round trips after warm-up")
    args = parser.parse_args()
    helper_path = args.helper.resolve()
    def helper_hash():
        with helper_path.open("rb") as binary:
            return hashlib.file_digest(binary, "sha256").hexdigest()
    binary_hash = helper_hash()
    folder = args.output_dir or Path(__file__).resolve().parents[2] / "target" / "hw-validation"
    folder.mkdir(parents=True, exist_ok=True)
    messages = queue.Queue()
    samples = []
    workers = []
    io_thread = None
    burst_latencies = []
    started_at = time.monotonic()
    with (folder / "helper-probe.stderr.log").open("w", encoding="utf8") as errors:
        child = subprocess.Popen([str(helper_path)], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=errors, text=True, encoding="utf8",
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        def reader():
            for line in child.stdout:
                messages.put(json.loads(line))
        threading.Thread(target=reader, daemon=True).start()
        try:
            ready = messages.get(timeout=45)
            if ready.get("event") != "ready":
                raise RuntimeError(ready)
            ready_ms = (time.monotonic() - started_at) * 1000
            resource_start = process_resources(child)
            measured_at = time.monotonic()
            for phase, duration in [("baseline", 5), ("load", 12), ("recovery", 5)]:
                if phase == "load":
                    # Exclude initialization/JIT and optional WMI metadata work
                    # from the steady-state helper CPU measurement.
                    resource_start = process_resources(child)
                    measured_at = time.monotonic()
                until = time.monotonic() + duration
                if phase == "load" and args.stress:
                    workers = [mp.Process(target=burn, args=(until,)) for _ in range(os.cpu_count() or 2)]
                    for worker in workers:
                        worker.start()
                    io_thread = threading.Thread(target=disk_load, args=(folder / f"probe-{uuid.uuid4()}.bin", until))
                    io_thread.start()
                while time.monotonic() < until:
                    request_id = len(samples) + 1
                    started = time.monotonic()
                    child.stdin.write(json.dumps({"id": request_id, "op": "snapshot"}) + "\n")
                    child.stdin.flush()
                    reply = messages.get(timeout=3)
                    if reply.get("id") != request_id or not reply.get("ok"):
                        raise RuntimeError(reply)
                    data = reply["data"]
                    latency_ms = (time.monotonic() - started) * 1000
                    resources = process_resources(child)
                    samples.append({"phase": phase, "latencyMs": latency_ms,
                        "sensorTimestampMs": data.get("timestampMs"), "resources": resources,
                        "sampleAgeMs": data.get("sampleAgeMs"), "storageAgeMs": data.get("storageAgeMs"),
                        "cpuTemp": (data.get("cpu") or {}).get("packageTempC"),
                        "cpuUsage": (data.get("cpu") or {}).get("totalUsage"),
                        "fans": [f.get("rpm") for f in data.get("fans", [])],
                        "gpus": len(data.get("gpus", [])), "disks": len(data.get("disks", []))})
                    time.sleep(0.25)
                print(f"{phase}: {sum(s['phase'] == phase for s in samples)} replies", flush=True)
            resource_end = process_resources(child)
            measured_seconds = time.monotonic() - measured_at
            for index in range(args.burst):
                request_id = len(samples) + index + 1
                started = time.monotonic()
                child.stdin.write(json.dumps({"id": request_id, "op": "snapshot"}) + "\n")
                child.stdin.flush()
                reply = messages.get(timeout=3)
                if reply.get("id") != request_id or not reply.get("ok"):
                    raise RuntimeError(reply)
                burst_latencies.append((time.monotonic() - started) * 1000)
            child.stdin.write(json.dumps({"id": len(samples) + args.burst + 1, "op": "shutdown"}) + "\n")
            child.stdin.flush()
            child.wait(timeout=10)
            if child.returncode:
                raise RuntimeError(f"helper exited {child.returncode}")
        finally:
            for worker in workers:
                worker.join(timeout=2)
                if worker.is_alive():
                    worker.terminate()
                    worker.join()
            if io_thread:
                io_thread.join(timeout=5)
            if child.poll() is None:
                child.kill()
                child.wait()
            (folder / "helper-probe.json").write_text(json.dumps(samples, indent=2), encoding="utf8")
    summary = {phase: {"samples": len(group), "maxLatencyMs": round(max(s["latencyMs"] for s in group), 2),
        "maxCpuUsage": max((s["cpuUsage"] for s in group if s["cpuUsage"] is not None), default=None),
        "cpuTemperatureSamples": sum(s["cpuTemp"] is not None for s in group),
        "maxSensorAgeMs": max(s["sampleAgeMs"] for s in group)}
        for phase in ["baseline", "load", "recovery"] if (group := [s for s in samples if s["phase"] == phase])}
    timestamps = sorted(set(s["sensorTimestampMs"] for s in samples if s["sensorTimestampMs"]))
    if helper_hash() != binary_hash:
        raise RuntimeError("Helper binary changed during measurement; rerun with an isolated build")
    summary = {"helperSha256": binary_hash, "stress": args.stress, "readyLatencyMs": round(ready_ms, 2), "phases": summary,
        "latencyMs": percentiles([s["latencyMs"] for s in samples]),
        "warmLatencyMs": percentiles([s["latencyMs"] for s in samples if s["phase"] != "baseline"]),
        "publicationIntervalMs": percentiles([b - a for a, b in zip(timestamps, timestamps[1:])]),
        "burstLatencyMs": percentiles(burst_latencies), "burstCount": len(burst_latencies),
        "steadyCpuPercentOneCore": round(100 * (resource_end["cpuSeconds"] - resource_start["cpuSeconds"]) / measured_seconds, 3),
        "maxWorkingSetMiB": round(max(s["resources"]["workingSetBytes"] for s in samples) / 1024 ** 2, 2),
        "maxPrivateMiB": round(max(s["resources"]["privateBytes"] for s in samples) / 1024 ** 2, 2)}
    (folder / "helper-summary.json").write_text(json.dumps(summary, indent=2), encoding="utf8")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
