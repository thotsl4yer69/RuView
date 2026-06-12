"""Ties a CSI source (the simulator, or a real RuView adapter) to a publisher.

Real-source seam
----------------
RuView's sensing-server emits per-node presence/vitals over its WebSocket/HTTP
API. To publish live data, implement an object with ``tick() -> list[(topic,
payload)]`` that subscribes to that stream and maps it onto the ``schemas``
builders, then pass it as ``source``. The publish path is identical.
"""

from __future__ import annotations

import logging
import time
from typing import Optional

from .config import BridgeConfig
from .simulator import CsiSimulator

log = logging.getLogger("sentient_bridge.bridge")


class SentientBridge:
    def __init__(self, config: BridgeConfig, publisher, source=None):
        self.cfg = config
        self.publisher = publisher
        self.source = source or CsiSimulator(nodes=config.nodes, seed=config.sim_seed)

    def run_once(self) -> int:
        messages = self.source.tick()
        for topic, payload in messages:
            self.publisher.publish(topic, payload, retain=False)
        return len(messages)

    def run(self, max_ticks: Optional[int] = None, sleep: bool = True) -> None:
        self.publisher.connect()
        try:
            n = 0
            while max_ticks is None or n < max_ticks:
                count = self.run_once()
                log.debug("published round", extra={"extra": {"tick": n, "messages": count}})
                n += 1
                if sleep:
                    time.sleep(self.cfg.publish_interval_sec)
        finally:
            self.publisher.close()
