"""Network helpers for the option tests (no third-party modules).

    python tests/broker/netutil.py

  127.0.0.1:18850  HTTP CONNECT proxy, no authentication
  127.0.0.1:18851  HTTP CONNECT proxy, Basic auth proxyuser / proxypass
  127.0.0.1:18852  silent listener: accepts TCP, never answers (connect timeout)
  127.0.0.1:18853  TCP forwarder to the broker (localhost:18840); while the file
                   target/netutil.freeze exists it drops every byte in both
                   directions, so a client's PUBLISH never reaches the broker

Every proxied connection is logged as "PROXY <port> CONNECT <target>", so a
test can check that its connection really went through the proxy.
"""

import base64
import socket
import sys
import threading

AUTH = "Basic " + base64.b64encode(b"proxyuser:proxypass").decode()
# also written to a file the WinCC OA test script reads
LOG_FILE = __import__("pathlib").Path(__file__).resolve().parents[2] / "target" / "netutil.log"
_log_lock = threading.Lock()


def log(msg):
    print(msg, flush=True)
    with _log_lock, open(LOG_FILE, "a", encoding="utf-8") as f:
        f.write(msg + "\n")


def pipe(src, dst):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        for s in (src, dst):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


FREEZE_FILE = LOG_FILE.parent / "netutil.freeze"


def frozen_pipe(src, dst):
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            if not FREEZE_FILE.exists():
                dst.sendall(data)
    except OSError:
        pass
    finally:
        for s in (src, dst):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def handle_forward(conn):
    try:
        up = socket.create_connection(("127.0.0.1", 18840), timeout=10)
        up.settimeout(None)
        log("FORWARD connected")
        threading.Thread(target=frozen_pipe, args=(conn, up), daemon=True).start()
        frozen_pipe(up, conn)
    except OSError as e:
        log(f"FORWARD ERROR {e}")
    finally:
        conn.close()


def handle_proxy(conn, port, need_auth):
    try:
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = conn.recv(4096)
            if not chunk:
                return
            head += chunk
        text = head.split(b"\r\n\r\n", 1)[0].decode("latin-1")
        lines = text.split("\r\n")
        method, target, _ = lines[0].split(" ", 2)
        headers = {}
        for line in lines[1:]:
            if ":" in line:
                k, v = line.split(":", 1)
                headers[k.strip().lower()] = v.strip()
        if method != "CONNECT":
            conn.sendall(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            return
        if need_auth and headers.get("proxy-authorization") != AUTH:
            log(f"PROXY {port} DENIED {target}")
            conn.sendall(b'HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="test"\r\n\r\n')
            return
        host, tport = target.rsplit(":", 1)
        up = socket.create_connection((host, int(tport)), timeout=10)
        up.settimeout(None)
        conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
        log(f"PROXY {port} CONNECT {target}")
        threading.Thread(target=pipe, args=(conn, up), daemon=True).start()
        pipe(up, conn)
    except Exception as e:  # noqa: BLE001 - test helper, keep serving
        log(f"PROXY {port} ERROR {e}")
    finally:
        conn.close()


def serve(port, handler):
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(50)
    while True:
        conn, _ = srv.accept()
        threading.Thread(target=handler, args=(conn,), daemon=True).start()


silent = []  # keep the sockets open


def handle_silent(conn):
    log("SILENT accepted")
    silent.append(conn)


def main():
    threading.Thread(target=serve, args=(18850, lambda c: handle_proxy(c, 18850, False)), daemon=True).start()
    threading.Thread(target=serve, args=(18851, lambda c: handle_proxy(c, 18851, True)), daemon=True).start()
    threading.Thread(target=serve, args=(18852, handle_silent), daemon=True).start()
    threading.Thread(target=serve, args=(18853, handle_forward), daemon=True).start()
    FREEZE_FILE.unlink(missing_ok=True)
    log("netutil ready: proxy 18850, proxy+auth 18851, silent 18852, forwarder 18853")
    threading.Event().wait()


if __name__ == "__main__":
    sys.exit(main())
