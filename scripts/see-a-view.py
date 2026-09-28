#!/usr/bin/env python3
"""See a view as a browser draws it, after its reads have had time to return.

    uv run --with websocket-client scripts/see-a-view.py "http://127.0.0.1:8777/#/m/hyperliquid/BTC" out.png 30

**Why not `chrome --headless --screenshot`.** It captures at the load event,
which a single-page screen reaches before any read returns; `--timeout` did not
delay it (Chrome exited in 1.1 s on 2026-09-28, and the chart read as
"reading..." for a reason that was the check's). `--virtual-time-budget` waits
for the network to go idle, and the status stream never does. So this drives
Chrome over the DevTools protocol: navigate, wait a real duration, capture,
and ask the page whether anything still reads "reading...".

A development tool, not a gate: it needs Chrome and a running tower.
"""

import base64
import json
import subprocess
import sys
import time
import urllib.request

import websocket  # websocket-client

CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
PORT = 9333


def main(url: str, out: str, wait: float) -> int:
    chrome = subprocess.Popen(
        [CHROME, "--headless=new", "--disable-gpu", f"--remote-debugging-port={PORT}", f"--remote-allow-origins=http://127.0.0.1:{PORT}",
         "--user-data-dir=/tmp/see-a-view-profile", "--window-size=1500,1000", "about:blank"],  # fmt: skip
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        for _ in range(50):
            try:
                tabs = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/json"))
                break
            except OSError:
                time.sleep(0.2)
        else:
            print("Chrome did not open its DevTools port", file=sys.stderr)
            return 2
        ws = websocket.create_connection(next(t for t in tabs if t["type"] == "page")["webSocketDebuggerUrl"])
        sent = 0

        def call(method: str, **params):
            nonlocal sent
            sent += 1
            ws.send(json.dumps({"id": sent, "method": method, "params": params}))
            while True:
                msg = json.loads(ws.recv())
                if msg.get("id") == sent:
                    return msg.get("result", {})

        call("Page.enable")
        call("Emulation.setDeviceMetricsOverride", width=1500, height=1000, deviceScaleFactor=1, mobile=False)
        call("Page.navigate", url=url)
        time.sleep(wait)
        reading = call("Runtime.evaluate", expression="document.body.innerText.includes('reading…')")["result"]["value"]
        with open(out, "wb") as fh:
            fh.write(base64.b64decode(call("Page.captureScreenshot", format="png")["data"]))
        print(f"{out}: {'a panel is still reading' if reading else 'nothing is still reading'} after {wait:.0f} s")
        return 1 if reading else 0
    finally:
        chrome.terminate()


if __name__ == "__main__":
    if len(sys.argv) != 4:
        print("usage: see-a-view.py <url> <out.png> <seconds>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1], sys.argv[2], float(sys.argv[3])))
