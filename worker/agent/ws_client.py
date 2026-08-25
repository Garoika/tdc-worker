import asyncio
import json
import logging
import websockets
from websockets.exceptions import ConnectionClosed

from typing import Any
from agent.config import HEARTBEAT_INTERVAL, LOG_TAIL_LINES
from agent.metrics import SystemMetrics
from agent.log_streamer import LogStreamer
from agent.native_auth_server import NativeAuthService
from agent.state_manager import state_manager

logger = logging.getLogger(__name__)

class WebSocketClient:
    def __init__(self, master_url: str, worker_token: str, docker_manager: Any, metrics_collector: SystemMetrics):
        self.master_url = master_url
        self.worker_token = worker_token
        self.docker = docker_manager
        self.metrics = metrics_collector
        self.log_streamer = LogStreamer(self.docker)
        self.native_auth = NativeAuthService(port=5000)
        self.auth_cancel_event = asyncio.Event()
        self.ws = None
        self.running = False
        self.tasks = []
        self.spawn_semaphore = asyncio.Semaphore(50)

    async def connect(self):
        headers = {'Authorization': f'Bearer {self.worker_token}'} if self.worker_token else {}
        try:
            self.ws = await websockets.connect(
                self.master_url, 
                additional_headers=headers,
                ping_interval=30,
                ping_timeout=30
            )
        except TypeError:
            self.ws = await websockets.connect(
                self.master_url, 
                extra_headers=headers,
                ping_interval=30,
                ping_timeout=30
            )
        logger.info(f"Connected to {self.master_url}")


    async def run(self):
        self.running = True
        retry_delay = 1

        while self.running:
            try:
                await self.connect()
                state_manager.update_system_info(master_connected=True)
                retry_delay = 1
                
                # Sync state on connect
                containers = await self.docker.list_running_containers()
                await self.send({
                    "type": "SYNC_STATE",
                    "containers": containers
                })
                
                # Start background loops
                heartbeat_task = asyncio.create_task(self.heartbeat_loop())
                log_task = asyncio.create_task(self.log_monitor_loop())
                self.tasks = [heartbeat_task, log_task]
                
                await self.listen_loop()
                
            except (ConnectionClosed, ConnectionRefusedError, Exception) as e:
                logger.error(f"WebSocket error: {e}")
                state_manager.update_system_info(master_connected=False)
                self.cancel_tasks()
                if not self.running:
                    break
                
                logger.info(f"Reconnecting in {retry_delay}s...")
                await asyncio.sleep(retry_delay)
                retry_delay = min(retry_delay * 2, 30)

    async def listen_loop(self):
        async for message_str in self.ws:
            try:
                msg = json.loads(message_str)
                msg_type = msg.get('type')
                
                if msg_type == 'INIT':
                    worker_name = msg.get('name')
                    if worker_name:
                        state_manager.update_system_info(node_name=worker_name)
                        logger.info(f"Node identified as: {worker_name}")

                elif msg_type == 'SPAWN_CONTAINER':
                    asyncio.create_task(self.handle_spawn_container(msg))

                elif msg_type == 'STOP_CONTAINER':
                    asyncio.create_task(self.handle_stop_container(msg))

                elif msg_type == 'STOP_ALL_CONTAINERS':
                    asyncio.create_task(self.handle_stop_all_containers())

                elif msg_type == 'START_AUTH_QUEUE':
                    accounts = msg.get('accounts', [])
                    self.auth_cancelled = False
                    asyncio.create_task(self.process_auth_queue(accounts))

                elif msg_type == 'CANCEL_AUTH_QUEUE':
                    self.auth_cancelled = True
                    self.auth_cancel_event.set()
                    logger.info("Auth queue cancelled by server")
                    asyncio.create_task(self.native_auth.stop())

                elif msg_type == 'GET_CONTAINER_LOGS':
                    asyncio.create_task(self.handle_get_container_logs(msg))
                    
            except Exception as e:
                logger.error(f"Error processing message: {e}")

    async def handle_get_container_logs(self, msg: dict):
        req_id = msg.get('request_id')
        cid = msg.get('container_id')
        job_id = msg.get('job_id')
        login = msg.get('login')
        logs = await self.docker.get_container_logs(container_id=cid, job_id=job_id, login=login, tail=0)
        await self.send({
            "type": "CONTAINER_LOGS_RESPONSE",
            "request_id": req_id,
            "logs": logs
        })

    async def handle_spawn_container(self, msg: dict):
        async with self.spawn_semaphore:
            job_id = msg.get('job_id')
            try:
                cid = await self.docker.spawn_container(
                    job_id=job_id,
                    account=msg.get('account', {}),
                    target=msg.get('target', {}),
                    limits=msg.get('limits', {})
                )
                await self.send({
                    "type": "CONTAINER_EVENT",
                    "job_id": job_id,
                    "container_id": cid,
                    "event": "spawned"
                })
            except Exception as e:
                await self.send({
                    "type": "CONTAINER_EVENT",
                    "job_id": job_id,
                    "event": "spawn_failed",
                    "error": str(e)
                })

    async def handle_stop_container(self, msg: dict):
        cid = msg.get('container_id')
        job_id = msg.get('job_id')
        success = await self.docker.stop_container(cid, job_id=job_id)
        await self.send({
            "type": "CONTAINER_EVENT",
            "job_id": job_id,
            "container_id": cid,
            "event": "stopped" if success else "stop_failed"
        })

    async def handle_stop_all_containers(self):
        logger.info("Stopping all running containers on worker node")
        containers = await self.docker.list_running_containers()
        for c in containers:
            await self.docker.stop_container(container_id=c['container_id'])

    async def process_auth_queue(self, accounts: list):
        """Process a queue of accounts for Twitch Device Code authorization via pure Python HTTP server on port 5000."""
        self.auth_cancelled = False
        self.auth_cancel_event.clear()
        total = len(accounts)
        logger.info(f"Starting native auth queue for {total} account(s)")
        
        try:
            await self.native_auth.start()

            for idx, acc in enumerate(accounts, 1):
                if self.auth_cancelled:
                    logger.info("Auth queue cancelled, stopping")
                    break
                    
                acc_id = acc.get('id')
                login = acc.get('login')
                
                # Notify server of progress
                await self.send({
                    "type": "ACCOUNT_AUTH_PROGRESS",
                    "account_id": acc_id,
                    "login": login,
                    "current": idx,
                    "total": total
                })
                
                try:
                    auth_result = await self.native_auth.authorize_account(
                        acc,
                        cancel_event=self.auth_cancel_event
                    )
                    
                    await self.send({
                        "type": "ACCOUNT_AUTH_SUCCESS",
                        "account_id": acc_id,
                        "login": login,
                        "client_secret": auth_result["client_secret"],
                        "twitch_user_id": auth_result["twitch_user_id"],
                        "current": idx,
                        "total": total
                    })
                    logger.info(f"✨ Account {login} authorized successfully ({idx}/{total})")
                    
                except asyncio.CancelledError:
                    break
                except Exception as e:
                    if self.auth_cancelled:
                        break
                    logger.error(f"Auth error for {login}: {e}")
                    await self.send({
                        "type": "ACCOUNT_AUTH_FAILED",
                        "account_id": acc_id,
                        "login": login,
                        "current": idx,
                        "total": total,
                        "error": str(e)
                    })
        finally:
            await self.native_auth.stop()
            logger.info("✨ Auth queue processing complete! Native auth server stopped.")
            await self.send({
                "type": "ACCOUNT_AUTH_COMPLETE"
            })

    async def send(self, message: dict):
        if self.ws:
            try:
                await self.ws.send(json.dumps(message))
            except (ConnectionClosed, Exception) as e:
                logger.debug(f"Could not send message: {e}")


    def auto_detect_ip(self) -> str:
        try:
            from agent.config import WORKER_PUBLIC_IP
            if WORKER_PUBLIC_IP:
                return WORKER_PUBLIC_IP
        except Exception:
            pass
            
        try:
            import urllib.request
            ip = urllib.request.urlopen('https://api.ipify.org', timeout=3).read().decode('utf-8').strip()
            if ip:
                return ip
        except Exception:
            pass
            
        try:
            import socket
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(('8.8.8.8', 80))
            ip = s.getsockname()[0]
            s.close()
            if ip:
                return ip
        except Exception:
            pass
            
        return '127.0.0.1'

    async def heartbeat_loop(self):
        public_ip = self.auto_detect_ip()
        sync_counter = 0
        
        while True:
            try:
                metrics_data = self.metrics.to_dict()
                current_containers = await self.docker.get_running_container_count()
                state_manager.update_metrics(
                    cpu=metrics_data.get('cpu_usage_percent', 0.0),
                    ram_used=metrics_data.get('ram_used_mb', 0.0),
                    ram_total=metrics_data.get('ram_total_mb', 0.0)
                )
                await self.send({
                    "type": "HEARTBEAT",
                    "metrics": metrics_data,
                    "current_containers": current_containers,
                    "public_ip": public_ip
                })
                await self.docker.cleanup_dead_containers()

                sync_counter += 1
                if sync_counter >= 3:
                    sync_counter = 0
                    containers = await self.docker.list_running_containers()
                    await self.send({
                        "type": "SYNC_STATE",
                        "containers": containers
                    })
            except Exception as e:
                logger.error(f"Heartbeat error: {e}")
            await asyncio.sleep(HEARTBEAT_INTERVAL)

    async def log_monitor_loop(self):
        while True:
            try:
                containers = await self.docker.list_running_containers()
                for c in containers:
                    cid = c['container_id']
                    status = await self.docker.get_container_status(cid)
                    
                    labels = c.get('labels', {}) or {}
                    login = labels.get('tdc.login') or c.get('login') or ''
                    job_id = labels.get('tdc.job_id') or ''
                    account_id = labels.get('tdc.account_id') or ''
                    game_name = labels.get('tdc.game') or c.get('game', '')

                    if status in ['exited', 'dead']:
                        await self.send({
                            "type": "CONTAINER_EVENT",
                            "job_id": job_id,
                            "account_id": account_id,
                            "container_id": cid,
                            "event": "exited",
                            "status": "stopped"
                        })
                        continue
                        
                    telemetry = await self.log_streamer.process_container_logs(cid, LOG_TAIL_LINES, job_id=job_id, login=login)
                    if not login and telemetry:
                        login = telemetry.get('account_login') or ''

                    if telemetry or login:
                        if telemetry and login:
                            state_manager.record_telemetry(login, game_name, telemetry)
                        await self.send({
                            "type": "TELEMETRY_UPDATE",
                            "container_id": cid,
                            "container_name": c.get('name', ''),
                            "job_id": job_id,
                            "account_id": account_id,
                            "account_login": login,
                            "login": login,
                            "game": game_name,
                            "telemetry": telemetry or {}
                        })
                    await asyncio.sleep(0.1)
            except Exception as e:
                logger.error(f"Log monitor error: {e}")
            await asyncio.sleep(5)

    def cancel_tasks(self):
        for task in self.tasks:
            task.cancel()
        self.tasks = []

    def stop(self):
        self.running = False
        self.cancel_tasks()
        if self.ws:
            asyncio.create_task(self.ws.close())
