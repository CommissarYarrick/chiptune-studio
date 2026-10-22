"""Command line interface for the chiptune engine."""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import engine
import score


def _parser():
    parser = argparse.ArgumentParser(description="Create, inspect and render chiptune scores.")
    parser.add_argument("--json", action="store_true", help=argparse.SUPPRESS)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("describe", help="Describe the score format and engine.")

    new = sub.add_parser("new", help="Create a score.")
    new.add_argument("score")
    new.add_argument("--profile", choices=("chip4", "console8"), default="chip4")
    new.add_argument("--title", required=True)
    new.add_argument("--template", choices=("lanterns", "empty"), default="lanterns")

    for name in ("inspect", "validate"):
        command = sub.add_parser(name)
        command.add_argument("score")

    patch = sub.add_parser("patch", help="Apply RFC6902 operations to a score revision.")
    patch.add_argument("score")
    patch.add_argument("--ops", required=True)
    patch.add_argument("--out", required=True)
    patch.add_argument("--expected-sha256", required=True)

    render = sub.add_parser("render", help="Render a score to WAV, MIDI and JSON files.")
    render.add_argument("score")
    render.add_argument("--out", required=True)
    render.add_argument("--sample-rate", type=int, default=32000)
    render.add_argument("--from-bar", type=float, default=0)
    render.add_argument("--bars", type=float)
    render.add_argument("--no-stems", action="store_true")
    render.add_argument("--solo", help="Comma-separated track IDs.")

    compare = sub.add_parser("compare", help="Compare render manifests.")
    compare.add_argument("manifests", nargs="+")

    transcribe = sub.add_parser("transcribe", help="Transcribe a hum from a WAV file.")
    transcribe.add_argument("wav")
    transcribe.add_argument("--out", required=True)
    transcribe.add_argument("--bpm", type=float, default=120)
    transcribe.add_argument("--quantize", type=float, default=.25)
    transcribe.add_argument("--profile", choices=("chip4", "console8"), default="chip4")
    transcribe.add_argument("--melody-only", action="store_true")
    transcribe.add_argument("--title")

    studio = sub.add_parser("studio", help="Run the local browser studio.")
    studio.add_argument("--root", default=os.environ.get("CHIPTUNE_DATA_DIR", str(Path(__file__).resolve().parent / "data")))
    studio.add_argument("--port", type=int, default=8765)
    return parser


def main(argv=None):
    try:
        args = _parser().parse_args(argv)
        command = args.command
        if command == "describe":
            result = engine.describe()
        elif command == "new":
            result = engine.create(args.score, args.profile, args.title, args.template)
        elif command == "inspect":
            result = engine.inspect(args.score)
        elif command == "validate":
            result = score.validate(score.load(args.score))
        elif command == "patch":
            operations = json.loads(Path(args.ops).read_text(encoding="utf-8"))
            result = engine.patch(args.score, operations, args.out, args.expected_sha256)
        elif command == "render":
            solo = [item for item in args.solo.split(",") if item] if args.solo is not None else None
            result = engine.render(args.score, args.out, args.sample_rate, args.from_bar, args.bars,
                                   not args.no_stems, solo)
        elif command == "compare":
            result = engine.compare(args.manifests)
        elif command == "transcribe":
            import transcribe
            result = transcribe.import_take(args.wav, args.out, args.bpm, args.quantize, args.profile,
                                            arrange=not args.melody_only, title=args.title)
        else:
            import studio
            result = studio.serve(args.root, args.port)
        print(json.dumps(result, ensure_ascii=False, allow_nan=False))
        return 0
    except (ValueError, KeyError, TypeError, OSError, json.JSONDecodeError) as error:
        print(json.dumps({"ok": False, "error": str(error)}, ensure_ascii=False, allow_nan=False))
        return 2


if __name__ == "__main__":
    sys.exit(main())
