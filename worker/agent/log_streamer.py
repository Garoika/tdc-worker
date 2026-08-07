import re
import logging
from agent.docker_manager import DockerManager

logger = logging.getLogger(__name__)

class LogStreamer:
    def __init__(self, docker_manager: DockerManager):
        self.docker_manager = docker_manager
        self.progress_pattern = re.compile(r'(\d+)\s*/\s*(\d+)\s*(?:minutes watched|min|minutes)?', re.IGNORECASE)
        self.drop_pattern1 = re.compile(r'(?:Time based drops|Active drop|Mining drop|Drop)\s*[:\-]\s*(.+)', re.IGNORECASE)
        self.drop_pattern2 = re.compile(r'Drop\s+"([^"]+)"', re.IGNORECASE)
        self.streamer_pattern1 = re.compile(r'SendSpadeEvents accepted for "([^"]+)"', re.IGNORECASE)
        self.streamer_pattern2 = re.compile(r'(?:watching|channel|streamer)\s*[:\-]?\s*([a-zA-Z0-9_]{3,25})', re.IGNORECASE)

    def parse_logs(self, logs: str) -> dict:
        telemetry = {}
        for line in logs.splitlines():
            line_str = line.strip()
            if not line_str:
                continue
                
            prog_match = self.progress_pattern.search(line_str)
            if prog_match and ('minutes' in line_str.lower() or 'min' in line_str.lower()):
                current_min = int(prog_match.group(1))
                target_min = int(prog_match.group(2))
                telemetry['watched_minutes'] = current_min
                telemetry['target_minutes'] = target_min
                telemetry['current_minutes'] = current_min
                telemetry['required_minutes'] = target_min
                if target_min > 0:
                    telemetry['percentage'] = min(100, int(round((current_min / target_min) * 100)))

            drop_match = self.drop_pattern1.search(line_str) or self.drop_pattern2.search(line_str)
            if drop_match:
                d_name = drop_match.group(1).strip()
                d_name = d_name.strip(' "\'[]')
                if d_name and len(d_name) > 1:
                    telemetry['drop_name'] = d_name
                    telemetry['current_drop'] = d_name

            st_match = self.streamer_pattern1.search(line_str) or self.streamer_pattern2.search(line_str)
            if st_match:
                streamer = st_match.group(1).strip()
                if streamer and len(streamer) >= 3 and streamer.lower() not in ['channel', 'true', 'false', 'streamer']:
                    telemetry['active_streamer'] = streamer

        return telemetry

    async def process_container_logs(self, container_id: str, tail: int) -> dict:
        logs = await self.docker_manager.get_container_logs(container_id, tail=tail)
        return self.parse_logs(logs)
