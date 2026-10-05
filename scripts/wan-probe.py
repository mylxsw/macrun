#!/usr/bin/env python3
"""Seeded user-space UDP impairment, NOT a real WAN or kernel traffic shaper.
Each direction gets half the RTT, independent loss, and a serial bandwidth queue.
Reports actual UDP bytes received, including QUIC headers/retransmits, separately
from application body bytes. Never changes system network configuration.
"""
import heapq
import json
import os
import random
import select
import socket
import sys
import threading
import time
from probe_support import Services, percentiles


class Relay:
    def __init__(self, port, rtt, mbps, loss):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.socket.bind(("127.0.0.1", 0))
        self.address = f"127.0.0.1:{self.socket.getsockname()[1]}"
        self.server = ("127.0.0.1", port)
        self.worker = None
        self.rtt, self.rate, self.loss = rtt/2000, mbps*1e6/8, loss
        self.bytes = [0, 0]
        self.dropped = 0
        self.running = True
        self.thread = threading.Thread(target=self.run)
        self.thread.start()

    def run(self):
        rng = random.Random(215)
        queue, free, serial = [], [0, 0], 0
        while self.running:
            now = time.monotonic()
            while queue and queue[0][0] <= now:
                _, _, data, destination = heapq.heappop(queue)
                self.socket.sendto(data, destination)
            timeout = min(.01, max(0, queue[0][0]-now)) if queue else .01
            if not select.select([self.socket], [], [], timeout)[0]:
                continue
            data, source = self.socket.recvfrom(65535)
            direction = int(source == self.server)
            if not direction:
                self.worker = source
            destination = self.worker if direction else self.server
            self.bytes[direction] += len(data)
            if destination is None or rng.random() < self.loss or len(queue) >= 8192:
                self.dropped += 1
                continue
            free[direction] = max(time.monotonic(), free[direction]) + len(data)/self.rate
            serial += 1
            heapq.heappush(queue, (free[direction]+self.rtt, serial, data, destination))

    def close(self):
        self.running = False
        self.thread.join(timeout=2)
        assert not self.thread.is_alive()
        self.socket.close()


results = []
for rtt, mbps, loss in [(20,100,0), (100,100,.001), (200,10,.01)]:
    with Services(sys.argv[1]) as s:
        relay = Relay(s.port, rtt, mbps, loss)
        try:
            s.worker(relay.address)
            large = s.source / "large"
            large.write_bytes(os.urandom(16*1024*1024))
            first = s.sync()
            row = {"rtt_ms":rtt,"mbps_per_direction":mbps,"loss":loss,"first":first,"changes":[]}
            for optimized in (False, True):
                for sample in range(3):
                    with large.open("r+b") as f:
                        f.seek(1024)
                        old = f.read(1)
                        f.seek(1024)
                        f.write(bytes([old[0]^1]))
                    before = list(relay.bytes)
                    latency = []
                    def control():
                        start = time.perf_counter()
                        assert s.request("mcp.servers")["servers"] == []
                        latency.append((time.perf_counter()-start)*1000)
                    v = s.sync(optimized, control)
                    v.update(optimized=optimized, sample=sample, udp_bytes=[a-b for a,b in zip(relay.bytes,before)], control=percentiles(latency))
                    assert v["sync"]["bytes"] == (1024*1024 if optimized else 16*1024*1024), v
                    assert large.read_bytes() == (s.mirror / "large").read_bytes()
                    row["changes"].append(v)
            row["dropped_datagrams"] = relay.dropped
            results.append(row)
            print(json.dumps(row), file=sys.stderr, flush=True)
        finally:
            relay.close()
print(json.dumps({"environment":"user-space loopback UDP relay; seed=215; 16 MiB random file, one-byte mutation; three samples per mode", "profiles":results}, indent=2))
