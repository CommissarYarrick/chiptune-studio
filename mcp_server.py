"""Model Context Protocol server for the chiptune engine."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from mcp.server import MCPServer

import engine
import score


def _inside(root: Path, value: str) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise ValueError("Path must be nonempty")
    candidate = (root / value).resolve()
    if not candidate.is_relative_to(root):
        raise ValueError("Path must stay below the configured root")
    return candidate


def make_server(root: Path):
    root = root.resolve()
    server = MCPServer("chiptune")

    @server.tool()
    def chip_describe() -> dict:
        """Return the chiptune engine's profiles, instruments and score format."""
        return engine.describe()

    @server.tool()
    def chip_new_score(path: str, profile: str, title: str, template: str) -> dict:
        """Create a score under the configured root using a supported profile and template."""
        return engine.create(_inside(root, path), profile, title, template)

    @server.tool()
    def chip_read_score(path: str) -> dict:
        """Read a score and its track summary from below the configured root."""
        return engine.inspect(_inside(root, path))

    @server.tool()
    def chip_validate_score(path: str) -> dict:
        """Validate a score stored below the configured root."""
        return score.validate(score.load(_inside(root, path)))

    @server.tool()
    def chip_patch_score(path: str, operations: list[dict], out: str, expected_sha256: str) -> dict:
        """Apply score operations after checking the current score hash; write the revision below root."""
        return engine.patch(_inside(root, path), operations, _inside(root, out), expected_sha256)

    @server.tool()
    def chip_render_score(path: str, out: str, sample_rate: int = 32000, from_bar: float = 0,
                          bars: float | None = None, stems: bool = True,
                          solo: list[str] | None = None) -> dict:
        """Render a score to a directory below the configured root."""
        return engine.render(_inside(root, path), _inside(root, out), sample_rate, from_bar, bars, stems, solo)

    @server.tool()
    def chip_import_hum(audio_path: str, out: str, bpm: float = 120, quantize: float = .25,
                        profile: str = "chip4", arrange: bool = True) -> dict:
        """Transcribe a WAV file below root into a score below root."""
        import transcribe
        return transcribe.import_take(_inside(root, audio_path), _inside(root, out), bpm, quantize,
                                      profile, arrange=arrange)

    @server.tool()
    def chip_compare_renders(paths: list[str]) -> dict:
        """Compare render manifests stored below the configured root."""
        return engine.compare([_inside(root, path) for path in paths])

    @server.resource("chip://guide")
    def guide() -> str:
        """Provide the machine-readable guide to the chiptune engine."""
        return json.dumps(engine.describe(), ensure_ascii=False, allow_nan=False)

    return server


def main(argv=None):
    parser = argparse.ArgumentParser(description="MCP server for the chiptune engine.")
    parser.add_argument("--root", default=os.environ.get("CHIPTUNE_DATA_DIR", str(Path(__file__).resolve().parent / "data")))
    args = parser.parse_args(argv)
    make_server(Path(args.root)).run(transport="stdio")


if __name__ == "__main__":
    main()
