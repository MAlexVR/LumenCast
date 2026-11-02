#!/usr/bin/env python3
"""Screenshots of the built UI in real WebKitGTK (the engine Tauri uses on Linux).

Serves ui/dist on a loopback port (the UI falls back to its mock API outside
Tauri), optionally runs JS once rendered, and saves a PNG of the viewport.

    pnpm --dir ui build
    python3 scripts/ui-shot.py --out shot.png
    python3 scripts/ui-shot.py -W 760 -H 520 --js 'document.querySelector(".ready").click()' --out small.png

Needs python3-gobject with WebKit2 4.1. Set UI_SHOT_OFFSCREEN=1 to avoid opening a window.
"""
import argparse
import functools
import http.server
import json
import os
import threading

import gi

gi.require_version("WebKit2", "4.1")
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    csp = None

    def log_message(self, *args):
        pass

    def end_headers(self):
        if self.csp:
            self.send_header("Content-Security-Policy", self.csp)
        super().end_headers()


def serve(directory, csp=None):
    handler = functools.partial(type("Handler", (QuietHandler,), {"csp": csp}), directory=directory)
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd.server_address[1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-W", "--width", type=int, default=1280)
    ap.add_argument("-H", "--height", type=int, default=800)
    ap.add_argument("--js", default="", help="JS run after the page has rendered")
    ap.add_argument("--wait", type=int, default=1500, help="ms before running --js")
    ap.add_argument("--after", type=int, default=1200, help="ms between --js and the shot")
    ap.add_argument("--dist", default=os.path.join(ROOT, "ui", "dist"))
    ap.add_argument("--csp-from", help="tauri.conf.json whose app.security.csp is sent as a header")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    csp = None
    if args.csp_from:
        with open(args.csp_from) as conf:
            csp = json.load(conf)["app"]["security"]["csp"]
    port = serve(args.dist, csp)
    webview = WebKit2.WebView.new_with_context(WebKit2.WebContext.new_ephemeral())
    win = Gtk.OffscreenWindow() if os.environ.get("UI_SHOT_OFFSCREEN") else Gtk.Window()
    win.add(webview)
    win.set_default_size(args.width, args.height)
    win.show_all()
    webview.load_uri(f"http://127.0.0.1:{port}/index.html")

    def run_js():
        if args.js:
            webview.evaluate_javascript(args.js, -1, None, None, None, None, None)
        GLib.timeout_add(args.after, snap)
        return False

    def snap():
        def done(view, res):
            view.get_snapshot_finish(res).write_to_png(args.out)
            print("saved", args.out)
            Gtk.main_quit()

        webview.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, WebKit2.SnapshotOptions.NONE, None, done)
        return False

    GLib.timeout_add(args.wait, run_js)
    GLib.timeout_add(args.wait + args.after + 20000, Gtk.main_quit)
    Gtk.main()


if __name__ == "__main__":
    main()
