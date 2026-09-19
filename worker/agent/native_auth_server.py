import asyncio
import logging
import urllib.parse
import aiohttp
from aiohttp import web
from typing import Optional, Dict, Any

logger = logging.getLogger('worker.native_auth')
logging.getLogger('aiohttp.access').setLevel(logging.WARNING)

CLIENT_ID = "kd1unb4b3q4t58fwlpcbzcbnm76a8fp"
SCOPES = "channel_read chat:read user_blocks_edit user_blocks_read user_follows_edit user_read"
AUTH_URL = f"https://id.twitch.tv/oauth2/authorize?client_id={CLIENT_ID}&redirect_uri=https://www.twitch.tv&response_type=token&scope={urllib.parse.quote(SCOPES)}"

class NativeAuthService:
    def __init__(self, port: int = 5000):
        self.port = port
        self.app = web.Application()
        self.runner: Optional[web.AppRunner] = None
        self.site: Optional[web.TCPSite] = None
        self.current_auth_state: Dict[str, Any] = {}
        self.is_running = False
        self._token_future: Optional[asyncio.Future] = None

        self.app.router.add_route('*', '/api/current', self._handle_current)
        self.app.router.add_route('POST', '/api/token', self._handle_token)
        self.app.router.add_route('*', '/api/skip', self._handle_skip)
        self.app.router.add_route('OPTIONS', '/{tail:.*}', self._handle_cors)

    async def start(self):
        """Start the local HTTP server on port 5000."""
        if self.is_running:
            return
        self.runner = web.AppRunner(self.app, access_log=None)
        await self.runner.setup()
        self.site = web.TCPSite(self.runner, '127.0.0.1', self.port)
        await self.site.start()
        self.is_running = True
        logger.info(f"⚡ Native Auth Server started on http://127.0.0.1:{self.port} for Chrome Extension (Android Client)")

    async def stop(self):
        """Stop the local HTTP server after briefly signaling finished status."""
        self.current_auth_state = {"status": "finished"}
        if self._token_future and not self._token_future.done():
            self._token_future.cancel()
        await asyncio.sleep(2.0)
        if self.site:
            await self.site.stop()
            self.site = None
        if self.runner:
            await self.runner.cleanup()
            self.runner = None
        self.is_running = False
        self.current_auth_state = {}
        logger.info("Native Auth Server stopped")

    _ALLOWED_ORIGINS = ("chrome-extension://", "http://127.0.0.1", "http://localhost", "https://www.twitch.tv", "https://id.twitch.tv")

    def _cors_headers(self, request: web.Request) -> dict:
        origin = request.headers.get("Origin", "")
        allowed = origin if any(origin.startswith(p) for p in self._ALLOWED_ORIGINS) else "http://127.0.0.1"
        return {
            "Access-Control-Allow-Origin": allowed,
            "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
            "Access-Control-Allow-Headers": "Content-Type, Authorization",
            "Cache-Control": "no-store, no-cache, must-revalidate"
        }

    async def _handle_cors(self, request: web.Request):
        return web.Response(status=200, headers=self._cors_headers(request))

    async def _handle_current(self, request: web.Request):
        if request.method == "OPTIONS":
            return await self._handle_cors(request)

        headers = self._cors_headers(request)
        if not self.current_auth_state:
            return web.json_response({"status": "waiting"}, headers=headers)

        return web.json_response(self.current_auth_state, headers=headers)

    async def _handle_token(self, request: web.Request):
        if request.method == "OPTIONS":
            return await self._handle_cors(request)

        headers = self._cors_headers(request)
        try:
            data = await request.json()
            access_token = data.get("access_token", "").strip()
            login = data.get("login", "")

            if not access_token:
                return web.json_response({"error": "No access_token provided"}, status=400, headers=headers)

            logger.info(f"📥 Received access_token from extension for {login or 'account'} ({len(access_token)} chars)")

            if self._token_future and not self._token_future.done():
                self._token_future.set_result(access_token)
                return web.json_response({"status": "ok", "message": "Token accepted"}, headers=headers)
            else:
                return web.json_response({"status": "ignored", "message": "No pending auth waiting for token"}, headers=headers)
        except Exception as e:
            logger.error(f"Error handling /api/token: {e}")
            return web.json_response({"error": str(e)}, status=500, headers=headers)

    async def _handle_skip(self, request: web.Request):
        if request.method == "OPTIONS":
            return await self._handle_cors(request)

        headers = self._cors_headers(request)
        logger.info("⏭️ Skip requested by user/extension")
        if self._token_future and not self._token_future.done():
            self._token_future.set_exception(Exception("Skipped by user"))
        return web.json_response({"status": "ok", "message": "Account skipped"}, headers=headers)

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
        cancel_event: asyncio.Event = None,
        timeout_seconds: int = 600
    ) -> Dict[str, str]:
        """
        Execute direct OAuth Authorization flow for Android App:
        1. Set auth state with auth_url and credentials for extension
        2. Wait for Chrome extension to approve and POST token to /api/token
        3. Validate token and return client_secret + twitch_user_id
        """
        login = account.get('login', '')
        auth_token = account.get('auth_token', '')
        password = account.get('password', '')

        loop = asyncio.get_event_loop()
        self._token_future = loop.create_future()

        self.current_auth_state = {
            "login": login,
            "index": login,
            "password": password,
            "auth_token": auth_token,
            "client_id": CLIENT_ID,
            "auth_url": AUTH_URL,
            "status": "pending"
        }

        logger.info(f"🔑 Auth requested for {login} via Android OAuth flow")

        async with aiohttp.ClientSession() as session:
            start_time = loop.time()
            access_token = None

            while loop.time() - start_time < timeout_seconds:
                if cancel_event and cancel_event.is_set():
                    self.current_auth_state = {"status": "cancelled"}
                    raise asyncio.CancelledError("Auth cancelled by user")

                if self._token_future.done():
                    access_token = self._token_future.result()
                    break

                await asyncio.sleep(0.5)

            if not access_token:
                self.current_auth_state = {"status": "timeout"}
                raise TimeoutError("Timed out waiting for Chrome extension authorization (10 min)")

            # Validate received token
            val_data = await self.validate_token(session, access_token)
            twitch_user_id = val_data.get("user_id", "")
            confirmed_login = val_data.get("login", login)
            validated_client_id = val_data.get("client_id", "")

            if validated_client_id != CLIENT_ID:
                logger.warning(f"Token client_id mismatch: got {validated_client_id}, expected {CLIENT_ID}")

            # Notify extension that this account is successfully authorized
            self.current_auth_state = {
                "status": "authorized",
                "login": confirmed_login
            }
            # Give extension 1.5s to read status and wipe session before next account
            await asyncio.sleep(1.5)

            return {
                "client_secret": access_token,
                "twitch_user_id": str(twitch_user_id),
                "login": confirmed_login
            }
