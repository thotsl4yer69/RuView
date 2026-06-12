"""CLI: ``python -m sentient_bridge``.

  --selftest    run the no-infrastructure dry-run self-test and exit
  --dry-run     simulate and print messages as JSON, no broker
  (default)     simulate and publish to the SENTIENT_MQTT_* broker
"""

from __future__ import annotations

import argparse
import logging
import sys

from .bridge import SentientBridge
from .config import BridgeConfig
from .publisher import DryRunPublisher, MqttPublisher


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="sentient_bridge",
                                 description="RuView CSI -> Sentient Core MQTT bridge")
    ap.add_argument("--selftest", action="store_true", help="run dry-run self-test and exit")
    ap.add_argument("--dry-run", action="store_true", help="print messages, do not connect")
    ap.add_argument("--ticks", type=int, default=None, help="stop after N rounds")
    args = ap.parse_args(argv)

    if args.selftest:
        from .selftest import run_selftest
        return 0 if run_selftest(verbose=True) else 1

    cfg = BridgeConfig.from_env()
    logging.basicConfig(level=getattr(logging, cfg.log_level.upper(), logging.INFO),
                        format="%(asctime)s %(levelname)s %(name)s: %(message)s")

    if args.dry_run:
        SentientBridge(cfg, DryRunPublisher(echo=True)).run(
            max_ticks=args.ticks if args.ticks is not None else 8, sleep=False)
        return 0

    if not cfg.mqtt_pass:
        logging.getLogger("sentient_bridge").warning(
            "SENTIENT_MQTT_PASS is not set; connecting without a password")
    bridge = SentientBridge(cfg, MqttPublisher(cfg))
    try:
        bridge.run(max_ticks=args.ticks)
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
