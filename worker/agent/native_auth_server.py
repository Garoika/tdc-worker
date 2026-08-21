import asyncio
import logging
import aiohttp
from aiohttp import web
from typing import Optional, Dict, Any

logger = logging.getLogger('worker.native_auth')

CLIENT_ID = "kd1unb4b3q4t58fwlpcbzcbnm76a8fp"
SCOPES = "channel_read chat:read user_blocks_edit user_blocks_read user_follows_edit user_read"

class NativeAuthService:
    def __init__(self, port: int = 5000):
        self.port = port
        self.app = web.Application()
        self.runner: Optional[web.AppRunner] = None
        self.site: Optional[web.TCPSite] = None
        self.current_auth_state: Dict[str, Any] = {}
        self.is_running = False

        self.app.router.add_route('*', '/api/current', self._handle_current)
        self.app.router.add_route('OPTIONS', '/{tail:.*}', self._handle_cors)

    async def start(self):
        """Start the local HTTP server on port 5000."""
        if self.is_running:
            return
        self.runner = web.AppRunner(self.app)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, '0.0.0.0', self.port)
        await self.site.start()
        self.is_running = True
        logger.info(f"⚡ Native Auth Server started on http://0.0.0.0:{self.port} for Chrome Extension")

    async def stop(self):
        """Stop the local HTTP server."""
        if self.site:
            await self.site.stop()
            self.site = None
        if self.runner:
            await self.runner.cleanup()
            self.runner = None
        self.is_running = False
        self.current_auth_state = {}
        logger.info("Native Auth Server stopped")

    async def _handle_cors(self, request: web.Request):
        return web.Response(
            status=200,
            headers={
                "Access-Control-Allow-Origin": "*",
                "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
                "Access-Control-Allow-Headers": "Content-Type"
            }
        )

    async def _handle_current(self, request: web.Request):
        if request.method == "OPTIONS":
            return await self._handle_cors(request)

        headers = {
            "Access-Control-Allow-Origin": "*",
            "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
            "Access-Control-Allow-Headers": "Content-Type",
            "Cache-Control": "no-store, no-cache, must-revalidate"
        }

        if not self.current_auth_state:
            return web.json_response({"status": "waiting"}, headers=headers)

        return web.json_response(self.current_auth_state, headers=headers)

    async def get_device_code(self, session: aiohttp.ClientSession) -> dict:
        """Request new device code from Twitch OAuth API."""
        url = "https://id.twitch.tv/oauth2/device"
        data = aiohttp.FormData()
        data.add_field("client_id", CLIENT_ID)
        data.add_field("scopes", SCOPES)

        async with session.post(url, data=data, timeout=aiohttp.ClientTimeout(total=10)) as resp:
            if resp.status != 200:
                text = await resp.text()
                raise Exception(f"Failed to get device code from Twitch: {resp.status} - {text}")
            return await resp.json()

    async def poll_token(self, session: aiohttp.ClientSession, device_code: str, timeout_seconds: int = 600, cancel_event: asyncio.Event = None) -> str:
        """Poll Twitch OAuth token endpoint until user authorizes via browser."""
        url = "https://id.twitch.tv/oauth2/token"
        start_time = asyncio.get_event_loop().time()

        while asyncio.get_event_loop().time() - start_time < timeout_seconds:
            if cancel_event and cancel_event.is_set():
                raise asyncio.CancelledError("Auth cancelled by user")

            await asyncio.sleep(2.5)

            data = aiohttp.FormData()
            data.add_field("client_id", CLIENT_ID)
            data.add_field("device_code", device_code)
            data.add_field("grant_type", "urn:ietf:params:oauth:grant-type:device_code")

            try:
                async with session.post(url, data=data, timeout=aiohttp.ClientTimeout(total=10)) as resp:
                    if resp.status == 200:
                        res = await resp.json()
                        return res.get("access_token")
                    elif resp.status in (400, 403):
                        # Still waiting for user authorization
                        continue
                    else:
                        text = await resp.text()
                        logger.debug(f"Twitch token poll status {resp.status}: {text}")
            except (aiohttp.ClientError, asyncio.TimeoutError):
                continue

        raise TimeoutError("Timed out waiting for Chrome extension authorization (10 min)")

    async def validate_token(self, session: aiohttp.ClientSession, access_token: str) -> dict:
        """Validate access token and retrieve user_id and login from Twitch."""
        url = "https://id.twitch.tv/oauth2/validate"
        headers = {"Authorization": f"OAuth {access_token}"}

        async with session.get(url, headers=headers, timeout=aiohttp.ClientTimeout(total=10)) as resp:
            if resp.status != 200:
                text = await resp.text()
                raise Exception(f"Failed to validate Twitch token: {resp.status} - {text}")
            return await resp.json()

    async def authorize_account(
        self,
        account: dict,
        cancel_event: asyncio.Event = None
    ) -> Dict[str, str]:
        """
        Execute full Device Code flow for a single account:
        1. Get device code
        2. Set state for Chrome extension on port 5000
        3. Poll Twitch until approved
        4. Validate token and return client_secret + twitch_user_id
        """
        login = account.get('login', '')
        auth_token = account.get('auth_token', '')
        password = account.get('password', '')

        async with aiohttp.ClientSession() as session:
            # 1. Request device code
            device_data = await self.get_device_code(session)
            device_code = device_data.get("device_code")
            user_code = device_data.get("user_code")

            logger.info(f"🔑 Auth requested for {login} | User Code: {user_code}")

            # 2. Expose to Chrome Extension
            self.current_auth_state = {
                "index": login,
                "password": password,
                "auth_token": auth_token,
                "device_code": device_code,
                "user_code": user_code,
                "client_id": CLIENT_ID,
                "state": "pending",
                "proxy": None
            }

            # 3. Poll for approval
            access_token = await self.poll_token(session, device_code, cancel_event=cancel_event)

            # 4. Validate token
            val_data = await self.validate_token(session, access_token)
            twitch_user_id = val_data.get("user_id", "")
            confirmed_login = val_data.get("login", login)

            # Clear state
            self.current_auth_state = {"status": "finished"}

            return {
                "client_secret": access_token,
                "twitch_user_id": str(twitch_user_id),
                "login": confirmed_login
            }
