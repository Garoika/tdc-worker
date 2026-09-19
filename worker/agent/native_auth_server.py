import asyncio
import logging
import aiohttp
from aiohttp import web
from typing import Optional, Dict, Any

logger = logging.getLogger('worker.native_auth')
logging.getLogger('aiohttp.access').setLevel(logging.WARNING)

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
        self._token_received: Optional[asyncio.Event] = None
        self._received_token: Optional[str] = None

        self.app.router.add_route('*', '/api/current', self._handle_current)
        self.app.router.add_route('*', '/api/token', self._handle_token)
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
        logger.info(f"⚡ Native Auth Server started on http://127.0.0.1:{self.port} for Chrome Extension")

    async def stop(self):
        """Stop the local HTTP server after briefly signaling finished status."""
        self.current_auth_state = {"status": "finished"}
        await asyncio.sleep(2.0)  # Allow Chrome extension poll cycle to receive 'finished' and wipe cookies/storage
        if self.site:
            await self.site.stop()
            self.site = None
        if self.runner:
            await self.runner.cleanup()
            self.runner = None
        self.is_running = False
        self.current_auth_state = {}
        logger.info("Native Auth Server stopped")

    _ALLOWED_ORIGINS = ("chrome-extension://", "http://127.0.0.1", "http://localhost")

    def _cors_origin(self, request: web.Request) -> str:
        origin = request.headers.get("Origin", "")
        if any(origin.startswith(p) for p in self._ALLOWED_ORIGINS):
            return origin
        return "http://127.0.0.1"

    def _cors_headers(self, request: web.Request) -> dict:
        return {
            "Access-Control-Allow-Origin": self._cors_origin(request),
            "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
            "Access-Control-Allow-Headers": "Content-Type",
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
        """Receive access_token from Chrome extension after OAuth redirect."""
        if request.method == "OPTIONS":
            return await self._handle_cors(request)

        headers = self._cors_headers(request)

        try:
            body = await request.json()
            token = body.get("access_token", "").strip()
        except Exception:
            return web.json_response({"error": "invalid body"}, status=400, headers=headers)

        if not token:
            return web.json_response({"error": "missing access_token"}, status=400, headers=headers)

        self._received_token = token
        if self._token_received:
            self._token_received.set()

        logger.info("🔑 Access token received from Chrome extension")
        return web.json_response({"ok": True}, headers=headers)

    async def validate_token(self, session: aiohttp.ClientSession, access_token: str) -> dict:
        """Validate access token and retrieve user_id and login from Twitch."""
        url = "https://id.twitch.tv/oauth2/validate"
        headers = {"Authorization": f"OAuth {access_token}"}

        async with session.get(url, headers=headers, timeout=aiohttp.ClientTimeout(total=10)) as resp:
            if resp.status != 200:
                text = await resp.text()
                raise Exception(f"Failed to validate Twitch token: {resp.status} - {text}")
            return await resp.json()

    def _build_authorize_url(self) -> str:
        """Build OAuth Implicit Grant authorize URL for Android App client."""
        return (
            f"https://id.twitch.tv/oauth2/authorize"
            f"?client_id={CLIENT_ID}"
            f"&redirect_uri=https://id.twitch.tv/oauth2/authorize"
            f"&response_type=token"
            f"&scope={SCOPES}"
        )

    async def authorize_account(
        self,
        account: dict,
        cancel_event: asyncio.Event = None
    ) -> Dict[str, str]:
        """
        Execute OAuth Implicit Grant flow for a single account:
        1. Set state with authorize_url for Chrome extension
        2. Wait for extension to POST access_token to /api/token
        3. Validate token and return client_secret + twitch_user_id
        """
        login = account.get('login', '')
        auth_token = account.get('auth_token', '')
        password = account.get('password', '')

        self._token_received = asyncio.Event()
        self._received_token = None

        authorize_url = self._build_authorize_url()

        logger.info(f"🔑 OAuth auth requested for {login}")

        # Expose to Chrome Extension
        self.current_auth_state = {
            "index": login,
            "login": login,
            "password": password,
            "auth_token": auth_token,
            "authorize_url": authorize_url,
            "client_id": CLIENT_ID,
            "state": "pending",
            "proxy": None
        }

        # Wait for extension to send the token (up to 10 min)
        timeout = 600
        try:
            if cancel_event:
                # Wait for either token or cancellation
                done, _ = await asyncio.wait(
                    [
                        asyncio.create_task(self._token_received.wait()),
                        asyncio.create_task(cancel_event.wait()),
                    ],
                    timeout=timeout,
                    return_when=asyncio.FIRST_COMPLETED
                )
                if cancel_event.is_set():
                    raise asyncio.CancelledError("Auth cancelled by user")
                if not done:
                    raise TimeoutError("Timed out waiting for Chrome extension authorization (10 min)")
            else:
                await asyncio.wait_for(self._token_received.wait(), timeout=timeout)
        except asyncio.TimeoutError:
            raise TimeoutError("Timed out waiting for Chrome extension authorization (10 min)")

        access_token = self._received_token
        if not access_token:
            raise Exception("No access token received")

        # Validate token
        async with aiohttp.ClientSession() as session:
            val_data = await self.validate_token(session, access_token)
            twitch_user_id = val_data.get("user_id", "")
            confirmed_login = val_data.get("login", login)
            token_client_id = val_data.get("client_id", "")

            if token_client_id != CLIENT_ID:
                logger.warning(f"⚠️ Token client_id mismatch: got {token_client_id}, expected {CLIENT_ID}")

        # Clear state
        self.current_auth_state = {"status": "finished"}

        return {
            "client_secret": access_token,
            "twitch_user_id": str(twitch_user_id),
            "login": confirmed_login
        }
