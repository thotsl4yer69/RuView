"""Payload + topic builders for the passive-space (WiFi-CSI) layer.

Schemas match ghost-fusion's README exactly. Presence is the safety-critical one:
``occupied`` is an explicit boolean with a ``confidence`` the fusion core gates
"confirmed empty" on (>= FUSION_MIN_EMPTY_CONF). Vitals are R&D / low-trust and
are world-state context only — the fusion core never scores them.
"""

from __future__ import annotations

from typing import Optional


def t_presence(node: str) -> str:
    return f"sentient/sensor/csi/{node}/presence"


def t_motion(node: str) -> str:
    return f"sentient/sensor/csi/{node}/motion"


def t_vitals(node: str) -> str:
    return f"sentient/sensor/csi/{node}/vitals"


def presence(occupied: bool, confidence: float, **extra) -> dict:
    """csi/{node}/presence — explicit occupancy evidence.

    ``occupied=False`` with high ``confidence`` is what lets the fusion engine
    *confirm* a room empty. Absence of this message is NOT emptiness — it ages
    the node to UNKNOWN downstream.
    """
    payload = {"occupied": bool(occupied), "confidence": round(float(confidence), 3)}
    payload.update(extra)
    return payload


def motion(moving: bool, energy: float, **extra) -> dict:
    """csi/{node}/motion — corroborating context (not a presence verdict)."""
    payload = {"moving": bool(moving), "energy": round(float(energy), 3)}
    payload.update(extra)
    return payload


def vitals(bpm_est: Optional[float] = None, **extra) -> dict:
    """csi/{node}/vitals — low-trust R&D context (e.g. breathing estimate).

    Never scored by the fusion core; carried only so the world-state snapshot
    can surface it.
    """
    payload = {}
    if bpm_est is not None:
        payload["bpm_est"] = bpm_est
    payload.update(extra)
    return payload
