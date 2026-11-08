#!/usr/bin/env python3
"""A tiny MQTT 3.1.1 broker for checking Sparkplug B from a board (QoS 0 only, no subscriptions routing).

It accepts CONNECT (with a will), prints every PUBLISH decoded as a Sparkplug B payload, and when a
client vanishes without DISCONNECT (closed socket or keep-alive timeout) prints the will like a real
broker would publish it. Usage: python sp_broker.py [port]   (default 1883)
"""
import asyncio
import sys
import time

DATATYPES = {3: "Int32", 4: "Int64", 8: "UInt64", 9: "Float", 11: "Boolean", 12: "String"}


def varint(b, i):
    v = shift = 0
    while True:
        x = b[i]
        i += 1
        v |= (x & 0x7F) << shift
        if x < 0x80:
            return v, i
        shift += 7


def fields(b):
    i = 0
    while i < len(b):
        key, i = varint(b, i)
        f, w = key >> 3, key & 7
        if w == 0:
            v, i = varint(b, i)
        elif w == 2:
            n, i = varint(b, i)
            v, i = bytes(b[i:i + n]), i + n
        elif w == 5:
            v, i = bytes(b[i:i + 4]), i + 4
        elif w == 1:
            v, i = bytes(b[i:i + 8]), i + 8
        else:
            raise ValueError(f"wire type {w}")
        yield f, w, v


def metric_text(raw):
    name = alias = dtype = None
    value = None
    for f, w, v in fields(raw):
        if f == 1:
            name = v.decode()
        elif f == 2:
            alias = v
        elif f == 4:
            dtype = v
        elif f == 10:
            value = v if v < 2**31 else v - 2**32
        elif f == 11:
            value = v
        elif f == 12:
            import struct
            value = round(struct.unpack("<f", v)[0], 6)
        elif f == 14:
            value = bool(v)
        elif f == 15:
            value = v.decode()
    ident = name if name is not None else f"alias {alias}"
    if name is not None and alias is not None:
        ident += f" (alias {alias})"
    return f"    {ident} : {DATATYPES.get(dtype, dtype)} = {value}"


def sparkplug_text(payload):
    ts = seq = None
    lines = []
    for f, w, v in fields(payload):
        if f == 1:
            ts = v
        elif f == 2:
            lines.append(metric_text(v))
        elif f == 3:
            seq = v
    head = f"  timestamp={ts} seq={seq}"
    return "\n".join([head] + lines)


def show(topic, payload, note=""):
    stamp = time.strftime("%H:%M:%S")
    print(f"[{stamp}] {topic} ({len(payload)} bytes){note}")
    if topic.startswith("spBv1.0/"):
        try:
            print(sparkplug_text(payload))
        except Exception as e:  # noqa: BLE001
            print(f"  cannot decode: {e}; hex={payload.hex()}")
    else:
        print(f"  {payload!r}")
    sys.stdout.flush()


async def read_packet(reader, timeout):
    first = await asyncio.wait_for(reader.readexactly(1), timeout)
    n = mult = 0
    mult = 1
    while True:
        x = (await reader.readexactly(1))[0]
        n += (x & 0x7F) * mult
        if x < 0x80:
            break
        mult *= 128
    body = await reader.readexactly(n) if n else b""
    return first[0], body


def text(b, i):
    n = int.from_bytes(b[i:i + 2], "big")
    return b[i + 2:i + 2 + n], i + 2 + n


async def client(reader, writer):
    peer = writer.get_extra_info("peername")
    will = None
    clean = False
    timeout = 90.0
    try:
        while True:
            head, body = await read_packet(reader, timeout)
            kind = head >> 4
            if kind == 1:
                name, i = text(body, 0)
                level, flags = body[i], body[i + 1]
                keep = int.from_bytes(body[i + 2:i + 4], "big")
                timeout = keep * 1.5 if keep else 3600
                i += 4
                cid, i = text(body, i)
                if flags & 0x04:
                    wt, i = text(body, i)
                    wp, i = text(body, i)
                    will = (wt.decode(), wp)
                print(f"[{time.strftime('%H:%M:%S')}] {peer} connected: client={cid.decode()} keepalive={keep}s will={'yes' if will else 'no'}")
                writer.write(b"\x20\x02\x00\x00")
            elif kind == 3:
                i = 0
                topic, i = text(body, 0)
                if (head >> 1) & 3:
                    i += 2
                show(topic.decode(), body[i:])
            elif kind == 8:
                writer.write(b"\x90\x03" + body[:2] + b"\x00")
            elif kind == 12:
                writer.write(b"\xd0\x00")
            elif kind == 14:
                clean = True
                print(f"[{time.strftime('%H:%M:%S')}] {peer} disconnected cleanly (will dropped)")
                break
            await writer.drain()
    except (asyncio.IncompleteReadError, asyncio.TimeoutError, ConnectionError):
        pass
    finally:
        if not clean and will:
            show(will[0], will[1], note="  <- WILL published by the broker")
        writer.close()


async def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 1883
    server = await asyncio.start_server(client, "0.0.0.0", port)
    print(f"sp_broker listening on port {port}")
    sys.stdout.flush()
    async with server:
        await server.serve_forever()


asyncio.run(main())
