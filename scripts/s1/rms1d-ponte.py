#!/usr/bin/env python3
"""Liga um socket unix deste Mac ao TCP do `adb forward`, para `mediad`, `robotctl` e `rms1d-roteiro` falarem com o
`rms1d` do robô pelo mesmo caminho do simulador. `mediad` só fala socket unix, e `socat` não vem neste Mac.

Uso: rms1d-ponte.py <socket-unix> <host:porta>
"""
import asyncio
import os
import sys


async def copiar(de, para):
    try:
        while True:
            pedaco = await de.read(65536)
            if not pedaco:
                break
            para.write(pedaco)
            await para.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        para.close()


async def principal(caminho, host, porta):
    async def atender(leitor, escritor):
        try:
            l2, e2 = await asyncio.open_connection(host, porta)
        except OSError as e:
            print(f"sem conexão com {host}:{porta}: {e}", file=sys.stderr, flush=True)
            escritor.close()
            return
        await asyncio.gather(copiar(leitor, e2), copiar(l2, escritor))

    # Socket velho de uma execução anterior: só o próprio caminho que esta ponte criou.
    if os.path.exists(caminho):
        os.unlink(caminho)
    servidor = await asyncio.start_unix_server(atender, caminho)
    print(f"{caminho} -> {host}:{porta}", flush=True)
    async with servidor:
        await servidor.serve_forever()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    h, p = sys.argv[2].rsplit(":", 1)
    try:
        asyncio.run(principal(sys.argv[1], h, int(p)))
    except KeyboardInterrupt:
        pass
