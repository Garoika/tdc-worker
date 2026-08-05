import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        # Matches: "45/60 minutes watched"
        self.progress_pattern = re.compile(r'(\d+)/(\d+) minutes watched')
        # Matches: "Time based drops : Rust"
        self.drop_pattern = re.compile(r'Time based drops\s*:\s*(.+)')
        # Matches: "SendSpadeEvents accepted for \"streamer\"" or "watching streamer"
        self.streamer_pattern1 = re.compile(r'SendSpadeEvents accepted for "([^"]+)"')
        self.streamer_pattern2 = re.compile(r'watching ([a-zA-Z0-9_]+)', re.IGNORECASE)

    def parse_logs(self, logs: str) -> dict:
        telemetry = {}
        for line in logs.splitlines():
            prog_match = self.progress_pattern.search(line)
            if prog_match:
                current_min = int(prog_match.group(1))
                target_min = int(prog_match.group(2))
                telemetry['watched_minutes'] = current_min
                telemetry['target_minutes'] = target_min
                telemetry['current_minutes'] = current_min
                telemetry['required_minutes'] = target_min
                if target_min > 0:
                    telemetry['progress_percent'] = (current_min / target_min) * 100
                    telemetry['percentage'] = round((current_min / target_min) * 100, 1)

            drop_match = self.drop_pattern.search(line)
            if drop_match:
                drop_name = drop_match.group(1).strip()
                telemetry['current_drop'] = drop_name
                telemetry['drop_name'] = drop_name

            st_match = self.streamer_pattern1.search(line) or self.streamer_pattern2.search(line)
            if st_match:
                streamer = st_match.group(1).strip()
                telemetry['active_streamer'] = streamer

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int) -> dict:
        logs = await self.docker_manager.get_container_logs(container_id, tail=tail)
        return self.parse_logs(logs)
