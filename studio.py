"""Local browser studio for recording, editing, and rendering chiptune scores."""
from __future__ import annotations

import base64
import importlib
import json
import mimetypes
import os
import sys
import uuid
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, urlparse

HERE = Path(__file__).resolve().parent
WEB = HERE / "web"
MAX_BODY = 20 * 1024 * 1024
MAX_WAV_SECONDS = 60

if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))
import engine  # noqa: E402


def serve(root, port=8765):
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=True)

    def confined(value):
        candidate = (root / value).resolve()
        if not candidate.is_relative_to(root):
            raise ValueError("Path is outside the project directory")
        return candidate

    def project_base(path):
        relative = Path(path).resolve().relative_to(root)
        parts = relative.parts
        if len(parts) >= 3 and parts[0] == "projects":
            return root / "projects" / parts[1]
        return Path(path).resolve().parent

    def recent_projects():
        projects_root = root / "projects"
        if not projects_root.is_dir() or projects_root.is_symlink() or not projects_root.resolve().is_relative_to(root):
            return []
        candidates = []
        for current, directories, filenames in os.walk(projects_root, followlinks=False):
            current_path = Path(current)
            directories[:] = [name for name in directories if not (current_path / name).is_symlink()]
            for name in filenames:
                if not name.lower().endswith(".json") or name.lower() in {"manifest.json", "timeline.json"} or name.lower().endswith(".transcription.json"):
                    continue
                candidate = current_path / name
                if candidate.is_symlink():
                    continue
                try:
                    resolved = candidate.resolve()
                    if not resolved.is_relative_to(root):
                        continue
                    candidates.append((candidate, candidate.stat().st_mtime_ns))
                except OSError:
                    continue
        valid = []
        for candidate, modified in sorted(candidates, key=lambda item: item[1], reverse=True)[:300]:
            try:
                inspection = engine.inspect(candidate)
                score = inspection["score"]
                valid.append((modified, {"path": candidate.resolve().relative_to(root).as_posix(),
                    "title": score["title"], "profile": score["profile"], "bpm": score["bpm"]}))
            except Exception:
                continue
        valid.sort(key=lambda item: item[0], reverse=True)
        return [record for _, record in valid[:30]]

    class Handler(BaseHTTPRequestHandler):
        server_version = "ChiptuneStudio/1.0"

        def _send(self, status, payload, content_type="application/json; charset=utf-8"):
            body = payload if isinstance(payload, bytes) else json.dumps(payload, ensure_ascii=False).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            self.wfile.write(body)

        def _json(self):
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0 or length > MAX_BODY:
                raise ValueError("Request is too large (20 MiB limit)")
            raw = self.rfile.read(length)
            try:
                return json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                raise ValueError("Invalid JSON") from error

        def _relative(self, path):
            return Path(path).resolve().relative_to(root).as_posix()

        def do_GET(self):
            parsed = urlparse(self.path)
            try:
                if parsed.path == "/":
                    return self._static("index.html")
                if parsed.path in {"/app.js", "/style.css", "/recorder.js"}:
                    return self._static(parsed.path[1:])
                if parsed.path == "/api/info":
                    return self._send(200, engine.describe())
                if parsed.path == "/api/projects":
                    return self._send(200, {"projects": recent_projects()})
                if parsed.path == "/api/read":
                    relative = parse_qs(parsed.query).get("path", [""])[0]
                    result = engine.inspect(confined(relative))
                    result["path"] = self._relative(result["path"])
                    return self._send(200, result)
                if parsed.path == "/audio":
                    relative = parse_qs(parsed.query).get("path", [""])[0]
                    audio_path = confined(relative)
                    if audio_path.suffix.lower() != ".wav" or not audio_path.is_file():
                        return self._send(404, {"error": "Audio not found"})
                    body = audio_path.read_bytes()
                    self.send_response(200)
                    self.send_header("Content-Type", "audio/wav")
                    self.send_header("Content-Length", str(len(body)))
                    self.send_header("X-Content-Type-Options", "nosniff")
                    self.end_headers()
                    return self.wfile.write(body)
                return self._send(404, {"error": "Resource not found"})
            except Exception as error:
                return self._send(400, {"error": str(error)})

        def _static(self, name):
            if name not in {"index.html", "app.js", "style.css", "recorder.js"}:
                return self._send(404, {"error": "Resource not found"})
            path = WEB / name
            if not path.is_file():
                return self._send(404, {"error": "Resource not found"})
            content_type = mimetypes.guess_type(name)[0] or "application/octet-stream"
            if content_type.startswith("text/") or content_type in {"application/javascript"}:
                content_type += "; charset=utf-8"
            return self._send(200, path.read_bytes(), content_type)

        def do_POST(self):
            origin = self.headers.get("Origin")
            allowed_origins = {f"http://127.0.0.1:{port}", f"http://localhost:{port}"}
            if origin and origin not in allowed_origins:
                return self._send(403, {"error": "Origin is not allowed"})
            try:
                data = self._json()
                if not isinstance(data, dict):
                    raise ValueError("Il corpo deve essere un oggetto JSON")
                if self.path == "/api/new":
                    project_id = uuid.uuid4().hex
                    path = confined(f"projects/{project_id}/score.json")
                    result = engine.create(path, profile=data.get("profile", "chip4"), title=data.get("title", "New composition"), template=data.get("template", "empty"))
                    result["path"] = result["project"] = self._relative(path)
                    return self._send(200, result)
                if self.path == "/api/transcribe":
                    encoded = data.get("wav_base64", "")
                    if not isinstance(encoded, str):
                        raise ValueError("Invalid wav_base64")
                    wav_bytes = base64.b64decode(encoded, validate=True)
                    if len(wav_bytes) > MAX_BODY:
                        raise ValueError("WAV exceeds the 20 MiB limit")
                    take_id = uuid.uuid4().hex
                    take_path = confined(f"takes/{take_id}.wav")
                    take_path.parent.mkdir(parents=True, exist_ok=True)
                    take_path.write_bytes(wav_bytes)
                    with wave.open(str(take_path), "rb") as source:
                        if source.getnchannels() != 1 or source.getsampwidth() != 2 or source.getframerate() <= 0:
                            raise ValueError("WAV must be mono PCM16")
                        if source.getnframes() / source.getframerate() > MAX_WAV_SECONDS:
                            raise ValueError("Recording exceeds 60 seconds")
                    project_id = uuid.uuid4().hex
                    score_path = confined(f"projects/{project_id}/score.json")
                    transcribe = importlib.import_module("transcribe")
                    result = transcribe.import_take(
                        audio_path=str(take_path), out=str(score_path), bpm=data.get("bpm", 120),
                        quantize=data.get("quantize", 0.25), profile=data.get("profile", "chip4"),
                        arrange=data.get("arrange", True), title=data.get("title", "Take"),
                    )
                    relative = self._relative(score_path)
                    inspection = engine.inspect(score_path)
                    inspection["path"] = inspection["project"] = relative
                    inspection["result"] = result
                    return self._send(200, inspection)
                if self.path == "/api/render":
                    score_path = confined(data.get("path", ""))
                    render_id = uuid.uuid4().hex
                    out = confined(data["out"]) if data.get("out") else confined(project_base(score_path) / "renders" / render_id)
                    manifest = engine.render(score_path, out, from_bar=data.get("from_bar", 0), bars=data.get("bars"))
                    mix = Path(manifest["mix"]).resolve()
                    manifest["audio_url"] = "/audio?path=" + quote(self._relative(mix))
                    return self._send(200, manifest)
                if self.path == "/api/patch":
                    score_path = confined(data.get("path", ""))
                    revision_id = uuid.uuid4().hex
                    out = confined(project_base(score_path) / "revisions" / f"{revision_id}.json")
                    result = engine.patch(score_path, data.get("operations"), out, data.get("expected_sha256"))
                    result["path"] = result["project"] = self._relative(out)
                    return self._send(200, result)
                return self._send(404, {"error": "Endpoint not found"})
            except Exception as error:
                return self._send(400, {"error": str(error)})

        def log_message(self, fmt, *args):
            print("studio:", fmt % args)

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=os.environ.get("CHIPTUNE_DATA_DIR", str(HERE / "data")))
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    serve(args.root, args.port)
