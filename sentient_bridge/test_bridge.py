"""Pytest wrapper around the CSI dry-run self-test plus schema checks.
Runs with no broker, no paho, no hardware."""

from sentient_bridge import schemas
from sentient_bridge.bridge import SentientBridge
from sentient_bridge.config import BridgeConfig
from sentient_bridge.publisher import DryRunPublisher
from sentient_bridge.selftest import run_selftest


def test_dry_run_selftest():
    assert run_selftest(verbose=False, raise_on_fail=True) is True


def test_presence_is_explicit_boolean_not_absence():
    pub = DryRunPublisher()
    SentientBridge(BridgeConfig(nodes=["lounge"]), pub).run_once()
    pres = [p for (t, p, _) in pub.messages if t.endswith("/presence")]
    assert pres and all(isinstance(p["occupied"], bool) for p in pres)
    assert all(0.0 <= p["confidence"] <= 1.0 for p in pres)


def test_occupied_to_empty_transition():
    pub = DryRunPublisher()
    bridge = SentientBridge(BridgeConfig(nodes=["lounge"]), pub)
    for _ in range(6):
        bridge.run_once()
    lounge = [p for (t, p, _) in pub.messages if t == schemas.t_presence("lounge")]
    assert lounge[0]["occupied"] is True
    assert lounge[-1]["occupied"] is False


def test_vitals_are_context_only_with_bpm():
    pub = DryRunPublisher()
    bridge = SentientBridge(BridgeConfig(nodes=["lounge"]), pub)
    for _ in range(3):  # occupied window -> vitals emitted
        bridge.run_once()
    vitals = [p for (t, p, _) in pub.messages if t.endswith("/vitals")]
    assert vitals and all("bpm_est" in p for p in vitals)


def test_topics_are_per_node_csi():
    pub = DryRunPublisher()
    SentientBridge(BridgeConfig(nodes=["lounge", "bedroom"]), pub).run_once()
    topics = pub.topics_seen()
    assert schemas.t_presence("lounge") in topics
    assert schemas.t_presence("bedroom") in topics
    assert all(t.startswith("sentient/sensor/csi/") for t in topics)


def test_deterministic_with_seed():
    a, b = DryRunPublisher(), DryRunPublisher()
    SentientBridge(BridgeConfig(sim_seed=7), a).run_once()
    SentientBridge(BridgeConfig(sim_seed=7), b).run_once()
    assert a.messages == b.messages
