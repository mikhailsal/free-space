FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
RUN apt-get update && apt-get install -y --no-install-recommends python3 time ca-certificates fio \
    && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["python3", "/harness/bench-v2-container.py", "--inside"]
