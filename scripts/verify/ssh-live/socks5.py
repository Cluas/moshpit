#!/usr/bin/env python3
"""A SOCKS5 proxy for the live checks (run.sh): no authentication, CONNECT
only, bytes relayed both ways. Binds a free port on 127.0.0.1, writes it to
<port file>, and serves until killed.

    socks5.py <port file>
"""
import asyncio
import socket
import struct
import sys


async def pipe(reader, writer):
    try:
        while True:
            data = await reader.read(65536)
            if not data:
                break
            writer.write(data)
            await writer.drain()
    except Exception:
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass


async def handle(reader, writer):
    try:
        _, count = await reader.readexactly(2)
        await reader.readexactly(count)
        writer.write(b"\x05\x00")
        await writer.drain()
        _, _, _, kind = await reader.readexactly(4)
        if kind == 1:
            host = socket.inet_ntoa(await reader.readexactly(4))
        elif kind == 3:
            host = (await reader.readexactly((await reader.readexactly(1))[0])).decode()
        else:
            host = socket.inet_ntop(socket.AF_INET6, await reader.readexactly(16))
        port = struct.unpack("!H", await reader.readexactly(2))[0]
        try:
            target_reader, target_writer = await asyncio.open_connection(host, port)
        except Exception:
            writer.write(b"\x05\x05\x00\x01\x00\x00\x00\x00\x00\x00")
            await writer.drain()
            writer.close()
            return
        writer.write(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
        await writer.drain()
        await asyncio.gather(pipe(reader, target_writer), pipe(target_reader, writer))
    except Exception:
        writer.close()


async def main(port_file):
    server = await asyncio.start_server(handle, "127.0.0.1", 0)
    with open(port_file, "w") as f:
        f.write(str(server.sockets[0].getsockname()[1]))
    async with server:
        await server.serve_forever()


asyncio.run(main(sys.argv[1]))
