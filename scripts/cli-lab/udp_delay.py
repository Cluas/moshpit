#!/usr/bin/env python3
"""A UDP relay that delays every datagram, both ways, by a fixed time: a
slow link between mosh-client and mosh-server on 127.0.0.1.

    udp_delay.py LISTEN_PORT SERVER_PORT ONE_WAY_MS

mosh-client talks to 127.0.0.1:LISTEN_PORT; the relay forwards to
127.0.0.1:SERVER_PORT from a socket of its own and sends the replies back
to the client's last address. Order is kept (a fixed delay).
"""
import heapq, select, socket, sys, time


def main():
    listen, server, delay = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]) / 1000
    front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    front.bind(("127.0.0.1", listen))
    back = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    back.bind(("127.0.0.1", 0))
    target = ("127.0.0.1", server)
    client = None
    queue = []  # (due, seq, sock, data, addr)
    seq = 0
    while True:
        now = time.monotonic()
        while queue and queue[0][0] <= now:
            _, _, sock, data, addr = heapq.heappop(queue)
            if addr is not None:
                try:
                    sock.sendto(data, addr)
                except OSError:
                    pass
        timeout = max(0.0, queue[0][0] - now) if queue else 1.0
        r, _, _ = select.select([front, back], [], [], timeout)
        for s in r:
            data, addr = s.recvfrom(65536)
            seq += 1
            if s is front:
                client = addr
                heapq.heappush(queue, (time.monotonic() + delay, seq, back, data, target))
            else:
                heapq.heappush(queue, (time.monotonic() + delay, seq, front, data, client))


if __name__ == "__main__":
    main()
