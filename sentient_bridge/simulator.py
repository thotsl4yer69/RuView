"""Hardware-free CSI presence simulator.

Models a small multi-node deployment and, crucially, an occupancy *transition*:
a node starts OCCUPIED (a person in the room) and then goes EMPTY. That is the
exact passive-layer evidence the GHOST fusion engine pairs with an unknown
active emitter to escalate to PLANTED_DEVICE — and dropping these messages must
read as UNKNOWN downstream, never EMPTY.
"""

from __future__ import annotations

import random
from typing import List, Tuple

from . import schemas


class CsiSimulator:
    def __init__(self, nodes=None, seed: int = 1312):
        self.nodes = list(nodes or ["lounge", "bedroom"])
        self._rng = random.Random(seed)
        self._tick = 0

    def _conf(self, base: float) -> float:
        return round(min(0.99, max(0.0, base + self._rng.uniform(-0.04, 0.04))), 3)

    def tick(self) -> List[Tuple[str, dict]]:
        out: List[Tuple[str, dict]] = []
        t = self._tick

        for i, node in enumerate(self.nodes):
            # First node: occupied for the first 3 ticks, then leaves (empty).
            # Other nodes: steadily empty. This gives the fusion engine a clean
            # occupied -> confirmed-empty transition to reason about.
            if i == 0:
                occupied = t < 3
            else:
                occupied = False
            conf = self._conf(0.9 if occupied else 0.85 if i == 0 else 0.8)
            out.append((schemas.t_presence(node), schemas.presence(occupied, conf)))

            # Motion corroborates but never decides presence on its own.
            energy = round(self._rng.uniform(0.3, 0.9) if occupied else self._rng.uniform(0.0, 0.15), 3)
            out.append((schemas.t_motion(node), schemas.motion(occupied, energy)))

            # Vitals only when occupied — low-trust context (breathing estimate).
            if occupied:
                out.append((schemas.t_vitals(node), schemas.vitals(
                    bpm_est=self._rng.randint(12, 18))))

        self._tick += 1
        return out
