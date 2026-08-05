FROM python:3.11-slim-bullseye

# Install docker CLI
RUN apt-get update && \
    apt-get install -y docker.io curl && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY agent/ ./agent/

ENTRYPOINT ["python", "-u", "-m", "agent.main"]
