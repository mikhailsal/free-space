FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends python3 time ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ENTRYPOINT ["python3", "/harness/bench-v2-container.py", "--inside"]
