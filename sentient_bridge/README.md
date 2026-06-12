# sentient_bridge — RuView CSI → Sentient Core / GHOST

Additive bridge that publishes RuView's **passive-space** (WiFi-CSI) presence
onto the `sentient/sensor/csi/*` MQTT topics consumed by the
[GHOST Fusion Engine](https://github.com/thotsl4yer69/ghost-fusion). It is the
passive half of the three-repo stack:

```
iNTERCEPT (active spectrum)  ─┐
                              ├─►  ghost-fusion  ─► sentient/threat/*
RuView (this repo, passive)  ─┘
```

## Topics published (schemas match ghost-fusion exactly)

| Topic | Meaning |
|---|---|
| `sentient/sensor/csi/{node}/presence` | `{occupied: bool, confidence: float}` — explicit occupancy evidence |
| `sentient/sensor/csi/{node}/motion` | `{moving: bool, energy: float}` — corroborating context |
| `sentient/sensor/csi/{node}/vitals` | `{bpm_est: ...}` — low-trust R&D context, **never scored** |

## The safety contract

Presence is the fusion engine's most safety-critical input. `occupied` is an
**explicit boolean with a confidence** — "empty" is *fresh positive evidence*,
never the absence of a message. If this bridge or a node stops publishing, the
fusion core ages the node out and the room collapses to **UNKNOWN**, never EMPTY,
so a dropped sensor can't manufacture a false planted-device alert. The simulator
exercises a full **occupied → confirmed-empty** transition for this reason.

## Run

```bash
python -m sentient_bridge --selftest          # no broker/paho/hardware; expect all checks passed
python -m sentient_bridge --dry-run --ticks 8 # watch the simulated CSI stream as JSON
pip install -r sentient_bridge/requirements.txt
export SENTIENT_MQTT_HOST=192.168.1.159 SENTIENT_MQTT_USER=sentient SENTIENT_MQTT_PASS=...
python -m sentient_bridge
```

## Configuration (env, shared with the stack)

`SENTIENT_MQTT_HOST` (192.168.1.159) · `SENTIENT_MQTT_PORT` (1883) ·
`SENTIENT_MQTT_USER` (sentient) · `SENTIENT_MQTT_PASS` (unset — set it) ·
`RUVIEW_BRIDGE_NODES` (`lounge,bedroom`) · `RUVIEW_BRIDGE_INTERVAL_SEC` (2.0) ·
`RUVIEW_BRIDGE_SIM_SEED` (1312).

## Wiring real RuView data

The bridge runs off a *source* with `tick() -> list[(topic, payload)]`; the
default is `CsiSimulator`. To publish live data, implement a source that
subscribes to the RuView sensing-server's presence/vitals stream and maps it onto
the `schemas` builders, then pass it to `SentientBridge(cfg, publisher, source=...)`.

---
Part of the GHOST / Sentient Core stack. Built on the upstream RuView project
(github.com/ruvnet/RuView, MIT); this module adds the MQTT bridge only and changes
none of the upstream code.
