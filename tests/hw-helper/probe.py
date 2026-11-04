"""Read-only helper protocol probe with optional bounded CPU/disk load.

Only snapshot/shutdown requests are sent; no fan writes or real cache cleanup.
Outputs measurements under target/hw-validation. Run explicitly, not in CI.
"""
import argparse
import json
import multiprocessing as mp
import os
from pathlib import Path
import queue
import subprocess
import threading
import time
import uuid


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
    args = parser.parse_args()
    folder = args.output_dir or Path(__file__).resolve().parents[2] / "target" / "hw-validation"
    folder.mkdir(parents=True, exist_ok=True)
    messages = queue.Queue()
    samples = []
    workers = []
    io_thread = None
    with (folder / "helper-probe.stderr.log").open("w", encoding="utf8") as errors:
        child = subprocess.Popen([str(args.helper.resolve())], stdin=subprocess.PIPE,
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
            for phase, duration in [("baseline", 5), ("load", 12), ("recovery", 5)]:
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
                    samples.append({"phase": phase, "latencyMs": (time.monotonic() - started) * 1000,
                        "sampleAgeMs": data.get("sampleAgeMs"), "storageAgeMs": data.get("storageAgeMs"),
                        "cpuTemp": data.get("cpu", {}).get("packageTempC"),
                        "cpuUsage": data.get("cpu", {}).get("totalUsage"),
                        "fans": [f.get("rpm") for f in data.get("fans", [])],
                        "gpus": len(data.get("gpus", [])), "disks": len(data.get("disks", []))})
                    time.sleep(0.25)
                print(f"{phase}: {sum(s['phase'] == phase for s in samples)} replies", flush=True)
            child.stdin.write(json.dumps({"id": len(samples) + 1, "op": "shutdown"}) + "\n")
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
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
