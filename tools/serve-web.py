#!/usr/bin/env python3
"""Development server for the browser build (target/web).

Serves the cross-origin isolation headers from web/_headers and answers the
page's POST /clear-site-data as web/functions/clear-site-data.js does. Map
data comes from the site's hosted build, whose bucket allows this server's
origins (see web/README.md).

    python3 tools/serve-web.py [--port 8787]
"""

import argparse
import http.server
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SITE = ROOT / "target" / "web"
ISOLATION = {
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Embedder-Policy": "require-corp",
    "Cross-Origin-Resource-Policy": "same-origin",
}


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".webmanifest": "application/manifest+json",
        ".js": "text/javascript",
        ".mjs": "text/javascript",
    }

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(SITE), **kwargs)

    def end_headers(self):
        for name, value in ISOLATION.items():
            self.send_header(name, value)
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_POST(self):
        # web/functions/clear-site-data.js
        if self.path == "/clear-site-data":
            self.send_response(204)
            self.send_header("Clear-Site-Data", '"cache", "storage"')
            self.end_headers()
        else:
            self.send_error(405)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8787)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"serving {SITE} at http://localhost:{args.port}/")
    server.serve_forever()


if __name__ == "__main__":
    main()
