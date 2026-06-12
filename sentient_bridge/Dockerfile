# Standalone image for the RuView CSI -> Sentient Core MQTT bridge.
# Build context is THIS directory (sentient_bridge/), so the image stays tiny
# and never pulls the full upstream Rust/Python workspace.
#
#   docker build -t mz1312/ruview-bridge ./sentient_bridge
#   docker run --rm -e SENTIENT_MQTT_HOST=... mz1312/ruview-bridge --dry-run
FROM python:3.12-slim

WORKDIR /app
COPY . ./sentient_bridge
RUN pip install --no-cache-dir paho-mqtt

ENV PYTHONUNBUFFERED=1
# Default: simulate WiFi-CSI presence and publish it to the broker.
ENTRYPOINT ["python", "-m", "sentient_bridge"]
