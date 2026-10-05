#!/usr/bin/env python3
"""Alternating pack/full-stream comparison. Both modes retain per-file fsync."""
import json
import sys
from probe_support import Services

with Services(sys.argv[1]) as s:
    s.worker()
    rows = []
    for sample, optimized in enumerate([False, True, True, False, False, True]):
        for i in range(1000):
            (s.source / f"{i:04}.txt").write_bytes(bytes([65+sample])*4096)
        value = s.sync(optimized)
        value.update(optimized=optimized, sample=sample)
        for i in range(1000):
            assert (s.mirror / f"{i:04}.txt").read_bytes() == bytes([65+sample])*4096
        rows.append(value)
    print(json.dumps({"dataset":"1000 compressible 4 KiB files; all changed each sample; alternating modes, release QUIC loopback","samples":rows}, indent=2))
