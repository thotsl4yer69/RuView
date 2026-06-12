"""Publishers: a real MQTT one (paho, lazy import) and a dry-run one that
captures messages in memory so the bridge is testable with no broker."""

from __future__ import annotations

import json
import logging

log = logging.getLogger("sentient_bridge.publisher")


class DryRunPublisher:
    """Captures (topic, payload, retain) instead of sending. ``echo=True`` also
    prints each message as a JSON line."""

    def __init__(self, echo: bool = False):
        self.messages: list[tuple[str, dict, bool]] = []
        self.echo = echo

    def publish(self, topic: str, payload: dict, retain: bool = False) -> None:
        self.messages.append((topic, payload, retain))
        if self.echo:
            print(json.dumps({"topic": topic, "payload": payload}))

    def topics_seen(self) -> set:
        return {t for (t, _, _) in self.messages}

    def connect(self) -> None:
        pass

    def close(self) -> None:
        pass


class MqttPublisher:
    """Publishes JSON to a broker; ``paho-mqtt`` imported lazily."""

    def __init__(self, config):
        self.cfg = config
        self._client = None

    def connect(self) -> None:
        import paho.mqtt.client as mqtt  # lazy

        try:
            client = mqtt.Client(
                mqtt.CallbackAPIVersion.VERSION2, client_id=self.cfg.mqtt_client_id
            )
        except AttributeError:
            client = mqtt.Client(client_id=self.cfg.mqtt_client_id)
        if self.cfg.mqtt_user:
            client.username_pw_set(self.cfg.mqtt_user, self.cfg.mqtt_pass or None)
        client.connect(self.cfg.mqtt_host, self.cfg.mqtt_port, keepalive=30)
        client.loop_start()
        self._client = client
        log.info("connected to broker", extra={"extra": self.cfg.redacted()})

    def publish(self, topic: str, payload: dict, retain: bool = False) -> None:
        if self._client is None:
            raise RuntimeError("MqttPublisher.connect() must be called first")
        self._client.publish(topic, json.dumps(payload), qos=0, retain=retain)

    def close(self) -> None:
        if self._client is not None:
            self._client.loop_stop()
            try:
                self._client.disconnect()
            except Exception:
                pass
