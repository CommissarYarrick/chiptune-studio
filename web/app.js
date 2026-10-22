"use strict";

const $ = selector => document.querySelector(selector);
const state = { profile: "chip4", quantize: 0, path: null, score: null, take: null, takeUrl: null, tapTimes: [], busy: false, dirty: false, originalBpm: null, recording: false, lockedButtons: new Map() };
const recorder = new TakeRecorder(level => drawWave(level), seconds => showTime(seconds), mode => recordingState(mode), blob => setTake(blob));

function setStatus(message, kind = "ready") {
  const node = $("#status"); node.className = `status ${kind}`; node.querySelector("span:last-child").textContent = message;
}
function markDirty() {
  state.dirty = true;
  $("#revisionHint").textContent = "Unsaved changes · save to create a revision.";
  $("#saveButton").classList.add("is-dirty");
}
function busy(value, message) {
  state.busy = value;
  if (value) {
    state.lockedButtons.clear();
    document.querySelectorAll("button").forEach(button => { state.lockedButtons.set(button, button.disabled); button.disabled = true; });
  } else {
    state.lockedButtons.forEach((disabled, button) => { button.disabled = disabled; });
    state.lockedButtons.clear();
    $("#recordButton").disabled = state.recording;
    $("#transcribeButton").disabled = !state.take || state.recording;
    $("#renderButton").disabled = !state.path;
    $("#saveButton").disabled = !state.path;
    $("#openProjectButton").disabled = !$("#recentProjects").value;
    $("#addNoteButton").disabled = !Object.keys(state.score?.score?.patterns || {}).length;
    syncPatternTools();
  }
  $("#layerInput").disabled = value || !state.path;
  $("#layerAtBar").disabled = value || !state.path || !$("#layerInput").checked;
  $("#wavFile").disabled = value || state.recording;
  $("#recentProjects").disabled = value || state.recording;
  if (message) setStatus(message, value ? "working" : "ready");
}
function showError(message, error) {
  $("#errorDetails").hidden = false;
  $("#errorText").textContent = error.message || String(error);
  setStatus(message, "error");
}
async function request(url, options = {}) {
  if (window.ChipSession) return window.ChipSession.request(url, options);
  const response = await fetch(url, options);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
  return data;
}
function jsonPost(url, body) { return request(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }); }
function title() { return $("#titleInput").value.trim() || "New track"; }
async function refreshProjects(selectedPath = state.path) {
  const data = await request("/api/projects");
  const select = $("#recentProjects");
  const projects = data.projects || [];
  select.innerHTML = '<option value="">Recent projects</option>' + projects.map(project =>
    `<option value="${escapeAttr(project.path)}">${escapeHtml(project.title)} · ${escapeHtml(project.profile)} · ${escapeHtml(project.bpm)} BPM</option>`).join("");
  if (selectedPath && projects.some(project => project.path === selectedPath)) select.value = selectedPath;
  $("#openProjectButton").disabled = !select.value;
}
function fileToBase64(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader(); reader.onload = () => resolve(String(reader.result).split(",")[1]); reader.onerror = () => reject(new Error("Could not read the file")); reader.readAsDataURL(blob);
  });
}
function showTime(seconds) { $("#recordTimer").textContent = `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`; }
function drawWave(level) {
  const bars = [...$("#waveform").children];
  const shift = Math.max(4, Math.min(100, 8 + level * 620));
  bars.forEach((bar, index) => { bar.style.height = `${Math.max(4, shift * (0.35 + Math.abs(Math.sin(index * 1.81 + Date.now() / 230)) * 0.65))}%`; });
}
function recordingState(mode) {
  const active = mode === "recording" || mode === "starting";
  state.recording = active;
  $("#recordButton").disabled = state.busy || mode === "starting";
  $("#recordButton").innerHTML = active ? "<span></span> Stop" : "<span></span> Record";
  $("#recordButton").classList.toggle("is-recording", active);
  $("#stopButton").disabled = mode !== "recording";
  $("#transcribeButton").disabled = active || state.busy || !state.take;
  $("#wavFile").disabled = active || state.busy;
  $("#recordLabel").textContent = mode === "recording" ? "Recording" : mode === "starting" ? "Accessing microphone…" : "Recording ready";
}
function setTake(blob) {
  if (state.takeUrl) URL.revokeObjectURL(state.takeUrl);
  state.take = blob; state.takeUrl = URL.createObjectURL(blob);
  $("#playTakeButton").disabled = false; $("#transcribeButton").disabled = false;
  $("#recordLabel").textContent = "Take ready";
  $("#transcribeButton").textContent = "Create music →";
  setStatus("Recording ready · press Create music", "ready");
}
function currentPatternId() { return $("#patternSelect").value; }
function selectedPattern() { return state.score?.score?.patterns?.[currentPatternId()]; }
function allNoteRows() {
  const rows = [];
  for (const [patternId, pattern] of Object.entries(state.score?.score?.patterns || {})) {
    pattern.notes.forEach((note, index) => rows.push({ patternId, pattern, note, index }));
  }
  return rows;
}
function renderScore() {
  const inspection = state.score;
  if (!inspection?.score) return;
  const score = inspection.score;
  $("#scoreName").textContent = score.title || "Project";
  $("#profileBadge").textContent = (score.profile || state.profile).toUpperCase().replace("CONSOLE8", "CONSOLE 8");
  const previousPattern = currentPatternId();
  $("#patternSelect").innerHTML = Object.keys(score.patterns || {}).map(id => `<option value="${escapeAttr(id)}">${escapeHtml(id)}</option>`).join("");
  if (score.patterns?.[previousPattern]) $("#patternSelect").value = previousPattern;
  $("#patternSelect").disabled = !Object.keys(score.patterns || {}).length;
  const rows = allNoteRows();
  $("#noteCount").textContent = `${rows.length} ${rows.length === 1 ? "NOTA" : "NOTE"}`;
  $("#notesBody").innerHTML = rows.length ? rows.map(({ patternId, note, index }) => `<tr data-pattern="${escapeAttr(patternId)}" data-index="${index}">
    <td class="note-id">${escapeHtml(patternId)} <span>#${index + 1}</span></td>
    <td><input class="cell-input pitch-input" aria-label="Note pitch" value="${escapeAttr(Array.isArray(note.note) ? note.note.join(", ") : note.note)}"></td>
    <td><input class="cell-input number-input" data-field="at" aria-label="Start in quarter beats" type="number" min="0" step="0.0625" value="${Number(note.at)}"></td>
    <td><input class="cell-input number-input" data-field="duration" aria-label="Duration in quarter beats" type="number" min="0.0625" step="0.0625" value="${Number(note.duration)}"></td>
    <td><input class="cell-input number-input" data-field="velocity" aria-label="Velocity" type="number" min="0.001" max="1" step="0.05" value="${Number(note.velocity)}"></td>
    <td><button class="remove-note" aria-label="Remove note">×</button></td></tr>`).join("") : '<tr><td colspan="6" class="empty-cell">This pattern is empty. Add a note to get started.</td></tr>';
  $("#patternSelect").onchange = () => { drawPianoRoll(); syncPatternTools(); };
  $("#addNoteButton").disabled = !Object.keys(score.patterns || {}).length;
  syncPatternTools();
  $("#saveButton").disabled = false;
  $("#renderButton").disabled = false;
  $("#revisionHint").textContent = state.dirty ? "Unsaved changes · save to create a revision." : "Changes are saved as a new revision.";
  $("#saveButton").classList.toggle("is-dirty", state.dirty);
  bindRows(); drawPianoRoll();
}
function escapeHtml(value) { return String(value ?? "").replace(/[&<>"']/g, ch => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch])); }
function escapeAttr(value) { return escapeHtml(value); }
function bindRows() {
  $("#notesBody").querySelectorAll("tr[data-pattern]").forEach(row => {
    const patternId = row.dataset.pattern; const index = Number(row.dataset.index);
    const update = (key, value) => { state.score.score.patterns[patternId].notes[index][key] = value; markDirty(); drawPianoRoll(); };
    const pitchInput = row.querySelector(".pitch-input");
    pitchInput.addEventListener("change", event => {
      try {
        const value = parsePitchInput(event.target.value);
        pitchInput.classList.remove("invalid"); update("note", value);
      } catch (error) {
        pitchInput.classList.add("invalid"); setStatus(error.message, "error");
      }
    });
    row.querySelectorAll("[data-field]").forEach(input => input.addEventListener("change", event => update(input.dataset.field, Number(event.target.value))));
    row.querySelector(".remove-note").addEventListener("click", () => { state.score.score.patterns[patternId].notes.splice(index, 1); markDirty(); renderScore(); });
  });
}
function midi(pitch) {
  if (Number.isInteger(pitch)) return pitch >= 0 && pitch <= 127 ? pitch : null;
  if (typeof pitch !== "string") return null;
  const trimmed = pitch.trim();
  if (/^-?\d+$/.test(trimmed)) {
    const numeric = Number(trimmed);
    return numeric >= 0 && numeric <= 127 ? numeric : null;
  }
  const match = /^([A-Ga-g])([#b]?)(-?\d+)$/.exec(trimmed);
  if (!match) return null;
  const semitone = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }[match[1].toUpperCase()] + (match[2] === "#" ? 1 : match[2] === "b" ? -1 : 0);
  const result = (Number(match[3]) + 1) * 12 + semitone;
  return result >= 0 && result <= 127 ? result : null;
}
function pitchTokens(value) {
  return Array.isArray(value) ? value : typeof value === "string" && value.includes(",") ? value.split(",").map(token => token.trim()) : [value];
}
function parsePitchInput(value) {
  const tokens = value.split(",").map(token => token.trim());
  if (!tokens.length || tokens.some(token => !token)) throw new Error("Enter a valid note or a chord separated by commas.");
  const parsed = tokens.map(token => {
    const numeric = /^-?\d+$/.test(token) ? Number(token) : null;
    const pitchValue = numeric === null ? midi(token) : (numeric >= 0 && numeric <= 127 ? numeric : null);
    if (pitchValue === null) throw new Error(`Invalid pitch: ${token}. Use MIDI 0–127 or notes such as C4, F#3, Bb2.`);
    return numeric === null ? token : pitchValue;
  });
  return parsed.length === 1 ? parsed[0] : parsed;
}
function syncPatternTools() {
  const pattern = selectedPattern();
  const disabled = !pattern?.notes?.length;
  $("#transposeDown").disabled = disabled;
  $("#transposeUp").disabled = disabled;
}
function drawPianoRoll() {
  const canvas = $("#pianoRoll"); const rect = canvas.getBoundingClientRect(); const scale = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, rect.width * scale); canvas.height = Math.max(1, rect.height * scale);
  const ctx = canvas.getContext("2d"); ctx.scale(scale, scale);
  const width = rect.width; const height = rect.height; const pattern = selectedPattern();
  ctx.fillStyle = "#121e30"; ctx.fillRect(0, 0, width, height);
  const notes = pattern?.notes || []; const length = Math.max(4, Number(pattern?.length || 16));
  const pitchValues = notes.flatMap(note => pitchTokens(note.note).map(midi).filter(value => value !== null)); const low = Math.max(0, Math.min(48, (pitchValues.length ? Math.min(...pitchValues) : 60) - 8)); const high = Math.min(127, Math.max(72, (pitchValues.length ? Math.max(...pitchValues) : 72) + 8));
  const rows = Math.max(1, high - low + 1); const rowHeight = height / rows;
  for (let p = low; p <= high; p++) {
    const y = height - (p - low + 1) * rowHeight;
    if ([1, 3, 6, 8, 10].includes(p % 12)) { ctx.fillStyle = "rgba(4,11,21,.3)"; ctx.fillRect(0, y, width, rowHeight); }
    ctx.strokeStyle = "rgba(226,219,198,.08)"; ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(width, y); ctx.stroke();
  }
  for (let beat = 0; beat <= length; beat++) {
    const x = beat / length * width;
    ctx.strokeStyle = beat % 4 === 0 ? "rgba(239,179,78,.28)" : "rgba(226,219,198,.08)";
    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, height); ctx.stroke();
  }
  notes.forEach((note, index) => {
    const values = pitchTokens(note.note).map(midi);
    const x = Number(note.at) / length * width; const w = Math.max(4, Number(note.duration) / length * width);
    values.forEach((noteValue, chordIndex) => {
      if (noteValue === null) return;
      const y = height - (noteValue - low + 1) * rowHeight + 1;
      ctx.fillStyle = index % 4 === 0 ? "#efb34e" : "#6dd2c1"; ctx.globalAlpha = .88;
      ctx.beginPath(); ctx.roundRect(x, y, Math.max(5, w), Math.max(3, rowHeight - 2), 3); ctx.fill(); ctx.globalAlpha = 1;
    });
  });
  if (!pitchValues.length) { ctx.fillStyle = "#91a0b5"; ctx.font = "12px system-ui"; ctx.fillText(notes.length ? "Invalid pitch" : "No notes in the selected pattern", 16, height / 2); }
}
function updateRevision(inspection) {
  state.path = inspection.project || inspection.path;
  state.score = inspection;
  state.dirty = false;
  const original = $("#originalAudio");
  const originalUrl = inspection.original_url || (inspection.result?.transcription && state.takeUrl);
  $("#originalPlayer").hidden = !originalUrl;
  if (originalUrl) { original.src = originalUrl; original.load(); }
  const faithfulUrl = inspection.faithful_url;
  $("#faithfulPlayer").hidden = !faithfulUrl;
  if (faithfulUrl) { $("#faithfulAudio").src = faithfulUrl; $("#faithfulAudio").load(); }
  $("#mixAudio").pause();
  $("#mixAudio").hidden = true;
  $("#downloadOgg").hidden = true;
  $("#mixName").textContent = "Press Update music to listen";
  if (inspection.score) {
    $("#titleInput").value = inspection.score.title || title();
    $("#bpmInput").value = inspection.score.bpm ?? 120;
    state.originalBpm = inspection.score.bpm ?? 120;
    state.profile = inspection.score.profile || state.profile;
    document.querySelectorAll("[data-profile]").forEach(button => button.classList.toggle("selected", button.dataset.profile === state.profile));
    $("#layerInput").disabled = state.busy;
    $("#layerAtBar").disabled = state.busy || !$("#layerInput").checked;
    renderScore();
  }
}
$("#layerInput").addEventListener("change", () => {
  $("#layerAtBar").disabled = !$("#layerInput").checked || state.busy;
});
$("#titleInput").addEventListener("input", () => { if (state.score) markDirty(); });
$("#bpmInput").addEventListener("change", event => {
  const bpm = Number(event.target.value);
  if (!Number.isFinite(bpm) || bpm < 30 || bpm > 300) {
    if (state.score) event.target.value = state.score.score.bpm;
    setStatus("BPM must be between 30 and 300.", "error"); return;
  }
  if (state.score) { state.score.score.bpm = bpm; markDirty(); }
});
$("#recentProjects").addEventListener("change", event => { $("#openProjectButton").disabled = !event.target.value; });
$("#openProjectButton").addEventListener("click", async () => {
  const path = $("#recentProjects").value;
  if (!path || state.busy) return;
  busy(true, "Opening project…");
  try {
    const inspection = await request(`/api/read?path=${encodeURIComponent(path)}`);
    updateRevision(inspection); setStatus(`Project opened · ${inspection.score.title}`, "success");
  } catch (error) { setStatus(`Could not open project: ${error.message}`, "error"); }
  finally { busy(false); }
});

async function stopRecording() {
  try { const blob = await recorder.stop(); if (blob) setTake(blob); }
  catch (error) { showError("Could not finish the recording. Try again.", error); }
}
$("#recordButton").addEventListener("click", async () => {
  if (state.busy) return;
  if (state.recording) { await stopRecording(); return; }
  $("#mixAudio").pause(); $("#originalAudio").pause(); $("#faithfulAudio").pause();
  try { await recorder.start(); setStatus("Recording · press Stop when finished", "working"); }
  catch (error) { showError("Could not access the microphone. Check browser permission.", error); recordingState("ready"); }
});
$("#stopButton").addEventListener("click", stopRecording);
$("#playTakeButton").addEventListener("click", () => { if (state.takeUrl) new Audio(state.takeUrl).play(); });
$("#wavFile").addEventListener("change", async event => {
  const file = event.target.files?.[0]; if (!file) return;
  if (file.size > 14 * 1024 * 1024) { setStatus("WAV exceeds the upload limit.", "error"); return; }
  try {
    const context = new AudioContext(); const decoded = await context.decodeAudioData(await file.arrayBuffer());
    const mono = new Float32Array(decoded.length);
    for (let channel = 0; channel < decoded.numberOfChannels; channel++) { const values = decoded.getChannelData(channel); for (let i = 0; i < values.length; i++) mono[i] += values[i] / decoded.numberOfChannels; }
    const blob = encodeWav(mono, decoded.sampleRate); await context.close();
    if (decoded.duration > 60) throw new Error("WAV exceeds 60 seconds.");
    setTake(blob); showTime(decoded.duration); setStatus(`WAV imported · ${file.name}`, "ready");
  } catch (error) { setStatus(`Could not import file: ${error.message}`, "error"); }
  event.target.value = "";
});
$("#transcribeButton").addEventListener("click", async () => {
  if (!state.take || state.busy || state.recording) return;
  const layering = $("#layerInput").checked && state.path;
  if (state.dirty) { $("#noteEditor").open = true; setStatus("Save your edited notes first. Your recording is still available.", "ready"); return; }
  const layerAtBar = Number($("#layerAtBar").value);
  if (layering && (!Number.isFinite(layerAtBar) || layerAtBar < 0)) { setStatus("Start bar must be zero or greater.", "error"); return; }
  busy(true, layering ? "Adding your recording to the song…" : "Creating music…");
  $("#errorDetails").hidden = true;
  let created = false;
  $("#transcribeButton").disabled = true;
  try {
    const result = await jsonPost("/api/transcribe", { wav_base64: await fileToBase64(state.take), bpm: Number($("#bpmInput").value), quantize: state.quantize, profile: state.profile, title: title(), arrange: false,
      ...(layering ? { layer_on: state.path, layer_at_bar: layerAtBar, expected_sha256: state.score.score_sha256 } : {}) });
    updateRevision(result);
    created = true;
    await refreshProjects(result.project).catch(() => {});
    await renderCurrentMix();
    setStatus("Music ready", "success");
    try { await $("#mixAudio").play(); } catch { setStatus("Music ready · press Play to listen", "success"); }
  } catch (error) {
    showError(created ? "Your song was saved, but playback could not be prepared. Press Update music to retry." : "Could not create music from this take. Your recording is preserved; please try again.", error);
  }
  finally { busy(false); $("#transcribeButton").disabled = !state.take; }

});
async function createProject(template = "empty") {
  if (state.busy) return;
  busy(true, "Creating project…");
  try { const result = await jsonPost("/api/new", { profile: state.profile, title: title(), template }); updateRevision(result); await refreshProjects(result.project); setStatus(template === "lanterns" ? "Example opened" : "New project ready", "success"); }
  catch (error) { setStatus(`Could not create project: ${error.message}`, "error"); }
  finally { busy(false); }
}
$("#newButton").addEventListener("click", () => createProject("empty"));
$("#exampleButton").addEventListener("click", () => createProject("lanterns"));
$("#addNoteButton").addEventListener("click", () => {
  const id = currentPatternId(); const pattern = selectedPattern(); if (!pattern) return;
  const end = Math.max(0, ...pattern.notes.map(note => Number(note.at) + Number(note.duration)));
  const at = Math.ceil((end - 1e-9) * 4) / 4;
  if (at + 0.25 > Number(pattern.length) + 1e-9) {
    $("#revisionHint").textContent = "Pattern is full: there is no room to add a ¼-beat note.";
    setStatus("Pattern is full: remove or shorten a note to add another.", "error"); return;
  }
  pattern.notes.push({ id: `note-${crypto.randomUUID().slice(0, 8)}`, at, note: "C4", duration: 0.25, velocity: 0.8 }); markDirty(); renderScore();
  const inputs = $("#notesBody").querySelectorAll("tr"); inputs[inputs.length - 1]?.scrollIntoView({ block: "nearest" });
});
function transposeSelected(semitones) {
  const pattern = selectedPattern();
  if (!pattern?.notes?.length) return;
  const converted = pattern.notes.map(note => pitchTokens(note.note).map(midi));
  if (converted.some(values => !values.length || values.some(value => value === null || !Number.isInteger(value) || value < 0 || value > 127))) {
    setStatus("Could not transpose: the pattern contains an invalid pitch.", "error"); return;
  }
  if (converted.some(values => values.some(value => value + semitones < 0 || value + semitones > 127))) {
    setStatus("Could not transpose: a note would fall outside the MIDI 0–127 range.", "error"); return;
  }
  pattern.notes.forEach((note, index) => {
    const shifted = converted[index].map(value => value + semitones);
    note.note = Array.isArray(note.note) || pitchTokens(note.note).length > 1 ? shifted : shifted[0];
  });
  markDirty(); renderScore(); setStatus(`${semitones > 0 ? "+" : ""}${semitones} semitones · save to create a revision`, "ready");
}
$("#transposeDown").addEventListener("click", () => transposeSelected(-12));
$("#transposeUp").addEventListener("click", () => transposeSelected(12));
$("#saveButton").addEventListener("click", async () => {
  if (!state.path || !state.score || state.busy) return;
  busy(true, "Saving a new revision…");
  try {
    const operations = [{ op: "replace", path: "/patterns", value: state.score.score.patterns }];
    if (title() !== state.score.score.title) operations.push({ op: "replace", path: "/title", value: title() });
    if (Number($("#bpmInput").value) !== state.originalBpm) operations.push({ op: "replace", path: "/bpm", value: Number($("#bpmInput").value) });
    const result = await jsonPost("/api/patch", { path: state.path, expected_sha256: state.score.score_sha256, operations });
    updateRevision(result); await refreshProjects(result.project); setStatus("Revision saved", "success");
  } catch (error) { setStatus(`Could not save revision: ${error.message}`, "error"); }
  finally { busy(false); }
});
async function renderCurrentMix() {
  const result = await jsonPost("/api/render", { path: state.path });
  $("#downloadOgg").hidden = !result.ogg_url;
  if (result.ogg_url) $("#downloadOgg").href = result.ogg_url;
  $("#mixAudio").src = result.audio_url;
  $("#mixAudio").hidden = false;
  $("#mixAudio").load();
  $("#mixName").textContent = `${result.title || state.score.score.title} · ${Number(result.seconds).toFixed(1)} s`;
}
$("#renderButton").addEventListener("click", async () => {
  if (!state.path || state.busy) return;
  if (state.dirty) { setStatus("Save the edited notes before rendering the mix.", "ready"); return; }
  busy(true, "Rendering mix…");
  try { await renderCurrentMix(); setStatus("Editable notes mix ready to play", "success"); }
  catch (error) { setStatus(`Could not render mix: ${error.message}`, "error"); }
  finally { busy(false); }
});
$("#tapButton").addEventListener("click", () => {
  const now = performance.now();
  if (state.tapTimes.length && now - state.tapTimes[state.tapTimes.length - 1] > 2500) state.tapTimes = [];
  state.tapTimes.push(now); state.tapTimes = state.tapTimes.slice(-7);
  if (state.tapTimes.length > 1) {
    const intervals = state.tapTimes.slice(1).map((time, index) => time - state.tapTimes[index]).sort((a, b) => a - b);
    const mid = Math.floor(intervals.length / 2); const median = intervals.length % 2 ? intervals[mid] : (intervals[mid - 1] + intervals[mid]) / 2;
    const bpm = Math.max(30, Math.min(300, Math.round(60000 / median))); $("#bpmInput").value = bpm; $("#bpmInput").dispatchEvent(new Event("change", { bubbles: true })); $("#tapReadout").textContent = `${bpm} BPM`;
  } else $("#tapReadout").textContent = "Tap again…";
});
document.querySelectorAll("[data-profile]").forEach(button => button.addEventListener("click", () => {
  state.profile = button.dataset.profile; document.querySelectorAll("[data-profile]").forEach(item => item.classList.toggle("selected", item === button));
  $("#profileBadge").textContent = state.profile.toUpperCase().replace("CONSOLE8", "CONSOLE 8");
  setStatus(`Profile for the next new track or take: ${state.profile.toUpperCase()}.`, "ready");
}));
document.querySelectorAll("[data-quantize]").forEach(button => button.addEventListener("click", () => {
  state.quantize = Number(button.dataset.quantize); document.querySelectorAll("[data-quantize]").forEach(item => item.classList.toggle("selected", item === button));
}));
$("#noteEditor").addEventListener("toggle", drawPianoRoll);
window.addEventListener("resize", drawPianoRoll);
setStatus("Ready to start");
refreshProjects().catch(error => setStatus(`Recent projects unavailable: ${error.message}`, "error"));
