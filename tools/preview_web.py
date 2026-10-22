"""Serve only the built static edition, including the GitHub Pages URL prefix."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

directory = Path(__file__).resolve().parents[1] / "docs"
prefix = "/chiptune-studio/"

class StaticEdition(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(directory), **kwargs)

    def do_GET(self):
        if not urlsplit(self.path).path.startswith(prefix):
            self.send_error(404)
            return
        self.path = self.path[len(prefix)-1:]
        super().do_GET()

if __name__ == "__main__":
    print("Browser edition: http://127.0.0.1:8770/chiptune-studio/", flush=True)
    ThreadingHTTPServer(("127.0.0.1", 8770), StaticEdition).serve_forever()
