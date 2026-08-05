FROM python:3.11-slim-bullseye

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY . .

ENV AUTH_TOKEN="" \
    PROXY_URL="" \
    TARGET_GAME="" \
    PRIORITY_STREAMERS="" \
    INSIDE_DOCKER="true"

ENTRYPOINT ["python", "-u", "main.py"]
