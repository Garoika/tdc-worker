import docker
import asyncio
import logging
import os
import json
import tempfile
from agent.config import DOCKER_IMAGE

logger = logging.getLogger(__name__)

class DockerManager:
    def __init__(self):
        self.client = docker.from_env()

    def ensure_farmer_image(self):
        """Verify and pull the latest Farmer Docker image on worker startup."""
        try:
            logger.info(f"Checking Farmer image: {DOCKER_IMAGE}...")
            try:
                local_img = self.client.images.get(DOCKER_IMAGE)
                img_id = local_img.short_id if hasattr(local_img, 'short_id') else local_img.id[:12]
                logger.info(f"Farmer image verified: {DOCKER_IMAGE} (ID: {img_id})")
            except docker.errors.ImageNotFound:
                logger.info(f"Farmer image {DOCKER_IMAGE} not found locally. Pulling latest from Docker Hub...")
                pulled = self.client.images.pull(DOCKER_IMAGE)
                p_id = pulled.short_id if hasattr(pulled, 'short_id') else pulled.id[:12]
                logger.info(f"Successfully pulled latest Farmer image: {p_id}")
        except Exception as e:
            logger.warning(f"Could not verify/pull Farmer image: {e}")

    async def spawn_container(self, job_id: str, account: dict, target: dict, limits: dict) -> str:
        login = account.get('login', f'job_{job_id[:8]}')
        name = f'tdc-farm-{job_id[:8]}'
        game = target.get('game', 'Rust')
        auth_token = account.get('auth_token', '')
        client_secret = account.get('client_secret', '')
        twitch_user_id = account.get('twitch_user_id', '')
        priority_streamers = target.get('priority_streamers', [])
        
        # Create temp config directory for this farm container
        temp_dir = tempfile.mkdtemp(prefix=f"tdc_farm_{login}_")
        config_file = os.path.join(temp_dir, f"config-{login}.json")
        
        config_data = {
            "FavouriteGames": [game],
            "TwitchSettings": {
                "TwitchUsers": [
                    {
                        "Enabled": True,
                        "AuthToken": auth_token,
                        "ClientSecret": client_secret,
                        "Id": twitch_user_id,
                        "Login": login,
                        "FavouriteGames": [game]
                    }
                ] if client_secret else [],
                "AvoidCampaign": [],
                "OnlyFavouriteGames": True,
                "ForceTryWithTags": True,
                "OnlyConnectedAccounts": False,
                "PriorityChannels": priority_streamers,
                "WatchManager": "WatchRequest"
            },
            "LaunchOnStartup": False,
            "LogLevel": 0,
            "WebhookURL": "",
            "WaitingSeconds": 300,
            "AttemptToWatch": 5,
            "WatchBrowserHeadless": True,
            "MinimizeInTray": False
        }
        
        with open(config_file, 'w', encoding='utf-8') as f:
            json.dump(config_data, f, indent=2)
            
        env = {
            'DOCKER_USER_ID': login,
            'DOCKER_GAME': game,
            'AUTH_TOKEN': auth_token,
            'CLIENT_SECRET': client_secret,
            'TWITCH_USER_ID': twitch_user_id,
            'PROXY_URL': account.get('proxy_url', ''),
            'INSIDE_DOCKER': 'true'
        }
        
        # CPU limits
        cpu_limit = limits.get('cpu_limit', 1.0)
        cpu_period = 100000
        cpu_quota = int(cpu_period * cpu_limit)
        
        # Memory limit
        mem_limit_mb = limits.get('memory_limit_mb', 512)
        mem_limit = f'{mem_limit_mb}m'
        
        labels = {
            'tdc.login': login,
            'tdc.job_id': str(job_id),
            'tdc.game': game,
            'tdc.account_id': str(account.get('id', ''))
        }
        
        logger.info(f"Spawning container {name} for user {login} watching {game}")
        loop = asyncio.get_running_loop()
        
        def _run():
            return self.client.containers.run(
                image=DOCKER_IMAGE,
                name=name,
                environment=env,
                labels=labels,
                volumes={
                    temp_dir: {'bind': '/app/Configuration', 'mode': 'rw'}
                },
                cpu_period=cpu_period,
                cpu_quota=cpu_quota,
                mem_limit=mem_limit,
                restart_policy={"Name": "on-failure", "MaximumRetryCount": 3},
                detach=True
            )
            
        try:
            container = await loop.run_in_executor(None, _run)
            return container.id
        except Exception as e:
            logger.error(f"Failed to spawn container {name}: {e}")
            raise

    async def stop_container(self, container_id: str = None, job_id: str = None) -> bool:
        loop = asyncio.get_running_loop()
        def _stop():
            targets = []
            if container_id:
                targets.append(container_id)
            if job_id:
                targets.append(f"tdc-farm-{job_id[:8]}")
                
            if not targets:
                logger.warning("stop_container called with no container_id or job_id")
                return True

            stopped_any = False
            for tid in targets:
                try:
                    container = self.client.containers.get(tid)
                    container.remove(force=True)
                    logger.info(f"Container {tid} forcibly stopped and removed")
                    stopped_any = True
                except docker.errors.NotFound:
                    pass
                except Exception as e:
                    logger.error(f"Error stopping container {tid}: {e}")
                    
            if not stopped_any and job_id:
                # Secondary search across all running containers
                try:
                    for c in self.client.containers.list(all=True):
                        if job_id[:8] in c.name or (container_id and container_id[:8] in c.id):
                            c.remove(force=True)
                            logger.info(f"Container {c.name} forcibly stopped via filter search")
                            stopped_any = True
                except Exception as e:
                    logger.error(f"Error searching containers for stop: {e}")
                    
            return True
        return await loop.run_in_executor(None, _stop)

    async def get_container_status(self, container_id: str) -> str:
        loop = asyncio.get_running_loop()
        def _status():
            try:
                container = self.client.containers.get(container_id)
                container.reload()
                return container.status
            except docker.errors.NotFound:
                return 'not_found'
            except Exception as e:
                logger.error(f"Error getting status for {container_id}: {e}")
                return 'unknown'
        return await loop.run_in_executor(None, _status)

    async def get_container_logs(self, container_id: str = None, job_id: str = None, tail: int = 0) -> str:
        loop = asyncio.get_running_loop()
        def _logs():
            targets = []
            if container_id:
                targets.append(container_id)
            if job_id:
                targets.append(f"tdc-farm-{job_id[:8]}")
                
            for tid in targets:
                try:
                    container = self.client.containers.get(tid)
                    tail_arg = "all" if tail <= 0 else tail
                    logs = container.logs(tail=tail_arg, stdout=True, stderr=True)
                    return logs.decode('utf-8', errors='replace')
                except docker.errors.NotFound:
                    pass
                except Exception as e:
                    return f"Error reading logs for {tid}: {e}"
            return "Container not found or has exited."
        return await loop.run_in_executor(None, _logs)

    async def list_running_containers(self) -> list:
        loop = asyncio.get_running_loop()
        def _list():
            containers = self.client.containers.list(filters={"status": "running"})
            res = []
            for c in containers:
                if c.name.startswith('tdc-farm-') or c.name.startswith('tdc-auth-'):
                    labels = c.labels or {}
                    env_list = c.attrs.get('Config', {}).get('Env', [])
                    env_dict = {}
                    for e in env_list:
                        if '=' in e:
                            k, v = e.split('=', 1)
                            env_dict[k] = v
                    login = labels.get('tdc.login') or env_dict.get('DOCKER_USER_ID', '')
                    if not login:
                        mounts = c.attrs.get('Mounts', [])
                        for m in mounts:
                            src = m.get('Source', '')
                            if 'tdc_farm_' in src:
                                parts = src.split('tdc_farm_')
                                if len(parts) > 1:
                                    login = parts[1].split('_')[0]
                                    break
                    game = labels.get('tdc.game') or env_dict.get('DOCKER_GAME', '')
                    res.append({
                        'container_id': c.id,
                        'name': c.name,
                        'status': c.status,
                        'login': login,
                        'game': game,
                        'labels': labels
                    })
            return res
        return await loop.run_in_executor(None, _list)

    async def get_running_container_count(self) -> int:
        loop = asyncio.get_running_loop()
        def _count():
            try:
                containers = self.client.containers.list(filters={"status": "running"})
                return len([c for c in containers if c.name.startswith('tdc-farm-') or c.name.startswith('tdc-auth-')])
            except Exception as e:
                logger.error(f"Error counting running containers: {e}")
                return 0
        return await loop.run_in_executor(None, _count)

    async def run_auth_container(self, account: dict, config_dir: str):
        """Launch a temporary auth container on port 5000 for Device Code Flow."""
        login = account.get('login', 'unknown')
        name = f'tdc-auth-{login}'
        
        loop = asyncio.get_running_loop()
        def _cleanup_old():
            try:
                c = self.client.containers.get(name)
                c.remove(force=True)
            except Exception:
                pass
        await loop.run_in_executor(None, _cleanup_old)
        
        proxy_info = account.get('proxy') or {}
        env = {
            'AUTH_TOKEN': account.get('auth_token', ''),
            'AUTH_PASSWORD': account.get('password', ''),
            'DOCKER_USER_ID': login,
            'DOCKER_GAME': 'Roblox',
            'INSIDE_DOCKER': 'true',
            'PROXY_HOST': str(proxy_info.get('host', '')),
            'PROXY_PORT': str(proxy_info.get('port', '')),
            'PROXY_USER': str(proxy_info.get('username', '')),
            'PROXY_PASS': str(proxy_info.get('password', '')),
            'PROXY_PROTOCOL': str(proxy_info.get('protocol', 'http'))
        }
        
        import platform
        is_linux = platform.system() == 'Linux'

        logger.info(f"Launching auth container {name} on port 5000")
        def _run():
            kwargs = {
                'image': DOCKER_IMAGE,
                'name': name,
                'environment': env,
                'volumes': {config_dir: {'bind': '/app/Configuration', 'mode': 'rw'}},
                'detach': True
            }
            if is_linux:
                kwargs['network_mode'] = 'host'
            else:
                kwargs['ports'] = {'5000/tcp': 5000}

            return self.client.containers.run(**kwargs)
            
        return await loop.run_in_executor(None, _run)

    async def cleanup_dead_containers(self):
        loop = asyncio.get_running_loop()
        def _clean():
            containers = self.client.containers.list(all=True, filters={"status": ["exited", "dead"]})
            for c in containers:
                if c.name.startswith('tdc-farm-') or c.name.startswith('tdc-auth-'):
                    logger.info(f"Removing dead container {c.name}")
                    try:
                        c.remove(force=True)
                    except Exception as e:
                        logger.error(f"Failed to remove {c.name}: {e}")
        await loop.run_in_executor(None, _clean)
