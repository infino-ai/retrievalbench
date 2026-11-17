#!/usr/bin/env python3
"""Sample the engine VM's CPU and the engine process's memory until a stop file appears, one
JSON line per sample, as the AWS setup's monitor recorded them:
  secs          the seconds this sample covers
  cpu_percent  the whole machine's busy CPU since the last sample, 0-100 over all cores
                (from /proc/stat, as the AWS setup measured it)
  process_cores the engine process's CPU since the last sample, in cores; the Infino bridge
                runs inside the JVM process, so its native threads are counted with it
  db_rss_mb     the engine process's resident memory (the AWS setup summed its Docker
                containers' memory, which was this process)
  db_vms_mb     the engine process's virtual memory
compare.py reads these lines.

usage: sampler.py <pid file> <out .jsonl> <stop file> [interval seconds]
"""
import json
import os
import sys
import time

DEFAULT_INTERVAL_SECS = 5.0
# The user and system CPU times in /proc/<pid>/stat, after the command name.
UTIME_FIELD, STIME_FIELD = 11, 12
# The `cpu` line of /proc/stat, after the label: the AWS setup summed the first seven
# columns (user through softirq) as the total and took the fourth (idle) as idle.
TOTAL_FIELDS = 7
IDLE_FIELD = 3


def machine_ticks():
    with open("/proc/stat") as f:
        fields = [int(v) for v in f.readline().split()[1:]]
    return sum(fields[:TOTAL_FIELDS]), fields[IDLE_FIELD]


def process_ticks(pid):
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    return int(fields[UTIME_FIELD]) + int(fields[STIME_FIELD])


def memory_mb(pid):
    out = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith(("VmRSS:", "VmSize:")):
                out[line.split(":")[0]] = int(line.split()[1]) / 1024
    return out.get("VmRSS", 0.0), out.get("VmSize", 0.0)


def main():
    pid_file, out, stop = sys.argv[1], sys.argv[2], sys.argv[3]
    interval = float(sys.argv[4]) if len(sys.argv) > 4 else DEFAULT_INTERVAL_SECS
    with open(pid_file) as f:
        pid = int(f.read().strip())
    ticks_per_sec = os.sysconf("SC_CLK_TCK")
    last_total, last_idle = machine_ticks()
    last_proc, last_time = process_ticks(pid), time.monotonic()
    with open(out, "a") as f:
        while not os.path.exists(stop):
            time.sleep(interval)
            total, idle = machine_ticks()
            proc, now = process_ticks(pid), time.monotonic()
            busy = (total - last_total) - (idle - last_idle)
            cpu = 100 * busy / (total - last_total) if total > last_total else 0.0
            cores = (proc - last_proc) / ticks_per_sec / (now - last_time)
            rss, vms = memory_mb(pid)
            f.write(json.dumps({"ts": time.time(), "secs": round(now - last_time, 3),
                                "cpu_percent": round(cpu, 1),
                                "process_cores": round(cores, 2),
                                "db_rss_mb": round(rss, 1), "db_vms_mb": round(vms, 1)}) + "\n")
            f.flush()
            last_total, last_idle, last_proc, last_time = total, idle, proc, now


if __name__ == "__main__":
    main()
