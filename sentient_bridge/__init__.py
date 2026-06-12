"""sentient_bridge — publish RuView WiFi-CSI presence onto the GHOST / Sentient
Core MQTT spine.

Additive module on top of the upstream RuView project (github.com/ruvnet/RuView,
MIT). It maps RuView's passive-space layer — per-node occupancy, motion and
(low-trust) vital-sign context — onto the ``sentient/sensor/csi/{node}/*`` topics
consumed by the GHOST Fusion Engine, using the exact schemas defined in
ghost-fusion's README.

The passive layer carries the fusion engine's most safety-critical input: a
*confirmed-empty* room. "Empty" must be fresh positive evidence, so this bridge
publishes presence as an explicit ``occupied``/``confidence`` reading — never an
absence. If the bridge (or a node) stops publishing, the fusion core ages the
node out and the room collapses to UNKNOWN on its side, never EMPTY.

Import-light: simulator + dry-run + self-test need neither ``paho-mqtt`` nor any
CSI hardware. ``paho-mqtt`` is imported lazily only for live publishing.
"""

__version__ = "0.1.0"
