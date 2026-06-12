"""Environment-driven config — mirrors ghost-fusion's SENTIENT_MQTT_* scheme."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import List, Optional


def _f(name: str, default: float) -> float:
    raw = os.environ.get(name)
    try:
        return float(raw) if raw not in (None, "") else default
    except ValueError:
        return default


def _i(name: str, default: int) -> int:
    raw = os.environ.get(name)
    try:
        return int(raw) if raw not in (None, "") else default
    except ValueError:
        return default


@dataclass(frozen=True)
class BridgeConfig:
    mqtt_host: str = "192.168.1.159"
    mqtt_port: int = 1883
    mqtt_user: str = "sentient"
    mqtt_pass: Optional[str] = None
    mqtt_client_id: str = "ruview-bridge"

    # CSI nodes this bridge represents (one RuView sensor per node).
    nodes: List[str] = field(default_factory=lambda: ["lounge", "bedroom"])

    publish_interval_sec: float = 2.0
    sim_seed: int = 1312
    log_level: str = "INFO"

    @classmethod
    def from_env(cls) -> "BridgeConfig":
        nodes_raw = os.environ.get("RUVIEW_BRIDGE_NODES", "lounge,bedroom")
        nodes = [n.strip() for n in nodes_raw.split(",") if n.strip()]
        return cls(
            mqtt_host=os.environ.get("SENTIENT_MQTT_HOST", "192.168.1.159"),
            mqtt_port=_i("SENTIENT_MQTT_PORT", 1883),
            mqtt_user=os.environ.get("SENTIENT_MQTT_USER", "sentient"),
            mqtt_pass=os.environ.get("SENTIENT_MQTT_PASS") or None,
            mqtt_client_id=os.environ.get("RUVIEW_BRIDGE_CLIENT_ID", "ruview-bridge"),
            nodes=nodes or ["lounge", "bedroom"],
            publish_interval_sec=_f("RUVIEW_BRIDGE_INTERVAL_SEC", 2.0),
            sim_seed=_i("RUVIEW_BRIDGE_SIM_SEED", 1312),
            log_level=os.environ.get("RUVIEW_BRIDGE_LOG_LEVEL", "INFO"),
        )

    def redacted(self) -> dict:
        return {
            "mqtt_host": self.mqtt_host,
            "mqtt_port": self.mqtt_port,
            "mqtt_user": self.mqtt_user,
            "mqtt_pass_set": bool(self.mqtt_pass),
            "mqtt_client_id": self.mqtt_client_id,
            "nodes": list(self.nodes),
            "publish_interval_sec": self.publish_interval_sec,
        }
