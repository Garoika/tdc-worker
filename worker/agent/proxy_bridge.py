import asyncio
import logging
import base64
from typing import Optional, Dict

logger = logging.getLogger(__name__)

class ProxyBridge:
    """
    Local proxy bridge that listens on 127.0.0.1:port as a standard HTTP proxy (no auth needed)
    and tunnels all traffic through any upstream proxy (SOCKS5, SOCKS4, HTTP/HTTPS with auth).
    This solves Chromium's lack of SOCKS5 authentication and transparently handles any upstream protocol.
    """
    def __init__(self, upstream: Dict, local_host: str = "0.0.0.0", local_port: int = 5001):
        self.upstream = upstream  # {'host': ..., 'port': ..., 'username': ..., 'password': ..., 'protocol': ...}
        self.local_host = local_host
        self.local_port = local_port
        self.server: Optional[asyncio.Server] = None

    async def start(self):
        self.server = await asyncio.start_server(
            self.handle_client, self.local_host, self.local_port
        )
        logger.info(
            f"🌐 Proxy Bridge active on {self.local_host}:{self.local_port} -> "
            f"{self.upstream.get('protocol', 'http')}://{self.upstream.get('host')}:{self.upstream.get('port')}"
        )

    async def stop(self):
        if self.server:
            self.server.close()
            await self.server.wait_closed()
            self.server = None
            logger.info("🛑 Proxy Bridge stopped")

    async def handle_client(self, client_reader: asyncio.StreamReader, client_writer: asyncio.StreamWriter):
        upstream_writer: Optional[asyncio.StreamWriter] = None
        try:
            initial_data = await client_reader.readuntil(b'\r\n')
            first_line = initial_data.decode('utf-8', errors='ignore').strip()
            
            if first_line.startswith('CONNECT '):
                parts = first_line.split(' ')
                target = parts[1]
                if ':' in target:
                    target_host, target_port_str = target.split(':', 1)
                    target_port = int(target_port_str)
                else:
                    target_host = target
                    target_port = 443

                # Read remaining HTTP headers
                while True:
                    line = await client_reader.readuntil(b'\r\n')
                    if line == b'\r\n':
                        break

                upstream_reader, upstream_writer = await self.connect_upstream(target_host, target_port)
                if not upstream_writer or not upstream_reader:
                    client_writer.write(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
                    await client_writer.drain()
                    return

                client_writer.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
                await client_writer.drain()

                await self.pipe(client_reader, client_writer, upstream_reader, upstream_writer)

            else:
                headers = [initial_data]
                while True:
                    line = await client_reader.readuntil(b'\r\n')
                    headers.append(line)
                    if line == b'\r\n':
                        break
                
                target_host = None
                target_port = 80
                for h in headers:
                    h_str = h.decode('utf-8', errors='ignore')
                    if h_str.lower().startswith('host:'):
                        host_val = h_str.split(':', 1)[1].strip()
                        if ':' in host_val:
                            target_host, p_str = host_val.split(':', 1)
                            target_port = int(p_str)
                        else:
                            target_host = host_val
                        break
                
                if not target_host:
                    return

                upstream_reader, upstream_writer = await self.connect_upstream(target_host, target_port)
                if not upstream_writer or not upstream_reader:
                    client_writer.write(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
                    await client_writer.drain()
                    return

                upstream_writer.write(b''.join(headers))
                await upstream_writer.drain()

                await self.pipe(client_reader, client_writer, upstream_reader, upstream_writer)

        except Exception as e:
            logger.debug(f"Bridge connection error: {e}")
        finally:
            try:
                client_writer.close()
                await client_writer.wait_closed()
            except Exception:
                pass
            if upstream_writer:
                try:
                    upstream_writer.close()
                    await upstream_writer.wait_closed()
                except Exception:
                    pass

    async def connect_upstream(self, target_host: str, target_port: int):
        u_host = str(self.upstream.get('host', '')).strip()
        u_port = int(self.upstream.get('port', 0))
        u_user = str(self.upstream.get('username', '')).strip()
        u_pass = str(self.upstream.get('password', '')).strip()
        u_proto = str(self.upstream.get('protocol', 'http')).lower()

        try:
            if 'socks5' in u_proto:
                return await self._connect_socks5(u_host, u_port, u_user, u_pass, target_host, target_port)
            elif 'socks4' in u_proto:
                return await self._connect_socks4(u_host, u_port, u_user, target_host, target_port)
            else:
                return await self._connect_http(u_host, u_port, u_user, u_pass, target_host, target_port)
        except Exception as e:
            logger.error(f"Failed to connect to upstream proxy ({u_proto}://{u_host}:{u_port}): {e}")
            return None, None

    async def _connect_socks5(self, u_host, u_port, u_user, u_pass, target_host, target_port):
        reader, writer = await asyncio.wait_for(asyncio.open_connection(u_host, u_port), timeout=10)
        
        # Greeting
        if u_user and u_pass:
            writer.write(b'\x05\x02\x00\x02')
        else:
            writer.write(b'\x05\x01\x00')
        await writer.drain()

        resp = await asyncio.wait_for(reader.readexactly(2), timeout=10)
        if resp[0] != 5:
            writer.close()
            return None, None

        method = resp[1]
        if method == 2:  # User/Pass auth
            u_b = u_user.encode('utf-8')
            p_b = u_pass.encode('utf-8')
            auth_msg = bytes([1, len(u_b)]) + u_b + bytes([len(p_b)]) + p_b
            writer.write(auth_msg)
            await writer.drain()
            
            auth_resp = await asyncio.wait_for(reader.readexactly(2), timeout=10)
            if auth_resp[1] != 0:
                logger.error(f"SOCKS5 upstream auth failed for user '{u_user}'")
                writer.close()
                return None, None
        elif method != 0:
            logger.error(f"SOCKS5 method {method} not supported by proxy")
            writer.close()
            return None, None

        # SOCKS5 CONNECT command (domain name addressing: 0x03)
        host_b = target_host.encode('utf-8')
        cmd = bytes([5, 1, 0, 3, len(host_b)]) + host_b + target_port.to_bytes(2, byteorder='big')
        writer.write(cmd)
        await writer.drain()

        reply = await asyncio.wait_for(reader.readexactly(4), timeout=10)
        if reply[1] != 0:
            logger.error(f"SOCKS5 CONNECT to {target_host}:{target_port} failed with code {reply[1]}")
            writer.close()
            return None, None

        atyp = reply[3]
        if atyp == 1:
            await reader.readexactly(4 + 2)
        elif atyp == 3:
            dlen = (await reader.readexactly(1))[0]
            await reader.readexactly(dlen + 2)
        elif atyp == 4:
            await reader.readexactly(16 + 2)

        return reader, writer

    async def _connect_socks4(self, u_host, u_port, u_user, target_host, target_port):
        reader, writer = await asyncio.wait_for(asyncio.open_connection(u_host, u_port), timeout=10)
        user_b = u_user.encode('utf-8') if u_user else b''
        host_b = target_host.encode('utf-8')
        msg = bytes([4, 1]) + target_port.to_bytes(2, byteorder='big') + bytes([0, 0, 0, 1]) + user_b + b'\x00' + host_b + b'\x00'
        writer.write(msg)
        await writer.drain()

        reply = await asyncio.wait_for(reader.readexactly(8), timeout=10)
        if reply[1] != 90:
            writer.close()
            return None, None

        return reader, writer

    async def _connect_http(self, u_host, u_port, u_user, u_pass, target_host, target_port):
        reader, writer = await asyncio.wait_for(asyncio.open_connection(u_host, u_port), timeout=10)
        connect_req = f"CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
        if u_user and u_pass:
            auth_b64 = base64.b64encode(f"{u_user}:{u_pass}".encode()).decode()
            connect_req += f"Proxy-Authorization: Basic {auth_b64}\r\n"
        connect_req += "\r\n"
        
        writer.write(connect_req.encode())
        await writer.drain()

        response_bytes = b""
        while b"\r\n\r\n" not in response_bytes:
            chunk = await asyncio.wait_for(reader.read(1024), timeout=10)
            if not chunk:
                break
            response_bytes += chunk

        if not response_bytes.startswith(b"HTTP/1.0 200") and not response_bytes.startswith(b"HTTP/1.1 200"):
            logger.error(f"HTTP upstream proxy rejected CONNECT: {response_bytes[:50]}")
            writer.close()
            return None, None

        return reader, writer

    async def pipe(self, r1: asyncio.StreamReader, w1: asyncio.StreamWriter, r2: asyncio.StreamReader, w2: asyncio.StreamWriter):
        async def _forward(reader, writer):
            try:
                while True:
                    data = await reader.read(65536)
                    if not data:
                        break
                    writer.write(data)
                    if writer.transport and writer.transport.get_write_buffer_size() > 131072:
                        await writer.drain()
            except Exception:
                pass
            finally:
                try:
                    writer.close()
                except Exception:
                    pass

        await asyncio.gather(_forward(r1, w2), _forward(r2, w1))
