"""Build the memory-only GitHub Pages edition. Does not publish or run CI."""
from pathlib import Path
import argparse
import os
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--no-build", action="store_true")
args = parser.parse_args()
if not args.no_build:
    subprocess.run(["cargo", "build", "--lib", "--release", "--no-default-features",
                    "--target", "wasm32-unknown-unknown"], cwd=root, check=True)
target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
out = root / "docs"
out.mkdir(exist_ok=True)
shutil.copy2(target / "wasm32-unknown-unknown/release/chiptune_studio.wasm", out / "chip.wasm")
for name in ["style.css", "app.js", "recorder.js", "browser-runtime.js", "wasm-worker.js"]:
    source = (root / "web" / name).read_text(encoding="utf-8")
    if name == "recorder.js":
        source = source.replace('"/recorder.js"', '"./recorder.js"')
        source = source.replace("open the studio on localhost", "use HTTPS or localhost")
    if name == "app.js":
        source = source.replace("Changes are saved as a new revision.", "Changes last only for this session. Download your song to keep it.")
        source = source.replace("Saving a new revision…", "Applying edits…").replace("Revision saved", "Edits applied for this session")
        source = source.replace("Unsaved changes · save to create a revision.", "Pending edits · apply them to this session.")
        source = source.replace("Your song was saved, but playback", "Your song is in this session, but playback")
    (out / name).write_text(source, encoding="utf-8")
html = (root / "web/index.html").read_text(encoding="utf-8")
html = html.replace('href="/"', 'href="./"').replace('href="/style.css"', 'href="./style.css"')
html = html.replace('src="/recorder.js"', 'src="./recorder.js"').replace('src="/app.js"', 'src="./app.js"')
html = html.replace('<script src="./app.js">', '<script src="./browser-runtime.js"></script>\n  <script src="./app.js">')
html = html.replace('id="recentProjects"', 'hidden id="recentProjects"').replace('id="openProjectButton"', 'hidden id="openProjectButton"')
html = html.replace('class="recent-projects"', 'hidden class="recent-projects"')
html = html.replace("LOCAL STUDIO", "BROWSER STUDIO")
html = html.replace("Audio stays on your computer.", "Session only. Nothing is saved or uploaded. Download before closing.")
html = html.replace("Save revision", "Apply edits").replace("Changes are saved as a new revision.", "Changes last only for this session.")
html = html.replace('<a id="downloadOgg"', '<a id="downloadWave" class="small-button" download="music.wav" hidden>Download WAV</a><a id="downloadSong" class="small-button" download="song.json" hidden>Download song</a><a id="downloadOgg"')
(out / "index.html").write_text(html, encoding="utf-8")
(out / ".nojekyll").write_text("", encoding="utf-8")
shutil.copy2(root / "LICENSE", out / "LICENSE.txt")
shutil.copy2(root / "licenses/THIRD_PARTY.txt", out / "THIRD_PARTY.txt")
print(f"Browser edition: {out} (no persistence, no backend, no CI)")
