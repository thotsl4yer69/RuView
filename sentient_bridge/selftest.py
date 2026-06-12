"""No-infrastructure self-test: drive the CSI simulator through the bridge with a
dry-run publisher and assert presence/motion/vitals are emitted with schemas the
GHOST fusion core accepts — including the occupied->empty transition. No broker,
no paho, no hardware."""

from __future__ import annotations

import sys

from . import schemas
from .bridge import SentientBridge
from .config import BridgeConfig
from .publisher import DryRunPublisher


def run_selftest(verbose: bool = True, raise_on_fail: bool = False) -> bool:
    cfg = BridgeConfig(nodes=["lounge", "bedroom"])
    pub = DryRunPublisher()
    bridge = SentientBridge(cfg, pub)
    for _ in range(6):  # spans the occupied(0-2) -> empty(3+) transition
        bridge.run_once()

    checks = []

    def check(ok, label):
        checks.append((bool(ok), label))

    presence = [(t, p) for (t, p, _) in pub.messages if t.endswith("/presence")]
    motion = [(t, p) for (t, p, _) in pub.messages if t.endswith("/motion")]
    vitals = [(t, p) for (t, p, _) in pub.messages if t.endswith("/vitals")]

    check(len(presence) > 0, "emits csi/{node}/presence")
    check(len(motion) > 0, "emits csi/{node}/motion")
    check(len(vitals) > 0, "emits csi/{node}/vitals")

    check(all("occupied" in p and "confidence" in p for _, p in presence),
          "presence has occupied + confidence")
    check(all(isinstance(p["occupied"], bool) for _, p in presence),
          "occupied is an explicit boolean (positive evidence, not absence)")
    check(all(0.0 <= p["confidence"] <= 1.0 for _, p in presence),
          "confidence in [0,1]")

    occ_vals = {p["occupied"] for _, p in presence}
    check(True in occ_vals and False in occ_vals,
          "presence shows an occupied -> empty transition")

    # The lounge node specifically must go from occupied to empty.
    lounge = [p for (t, p) in presence if t == schemas.t_presence("lounge")]
    check(lounge and lounge[0]["occupied"] is True and lounge[-1]["occupied"] is False,
          "lounge goes occupied -> confirmed empty")

    check(all("bpm_est" in p for _, p in vitals), "vitals carry bpm_est (context only)")
    # Topics are well-formed csi/{node}/... for configured nodes.
    nodes_ok = all(t.startswith("sentient/sensor/csi/") for (t, _, _) in pub.messages)
    check(nodes_ok, "all topics are sentient/sensor/csi/{node}/*")

    ok_all = all(ok for ok, _ in checks)
    if verbose:
        print("RuView sentient_bridge — dry-run self-test")
        for ok, label in checks:
            print(f"  [{'PASS' if ok else 'FAIL'}] {label}")
        print(f"\n{sum(1 for ok, _ in checks if ok)}/{len(checks)} checks passed")
    if raise_on_fail and not ok_all:
        raise AssertionError("bridge self-test failures: "
                             + "; ".join(label for ok, label in checks if not ok))
    return ok_all


if __name__ == "__main__":
    sys.exit(0 if run_selftest(verbose=True) else 1)
