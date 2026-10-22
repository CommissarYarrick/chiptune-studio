// Session-only adapter. Scores and recordings never leave this page or survive it.
(() => {
  const worker = new Worker(new URL("./wasm-worker.js", document.currentScript.src), {type:"module"});
  const pending = new Map();
  const blobs = new Set();
  let counter = 0;
  let current = null;
  function invoke(command, payload = {}) {
    return new Promise((resolve, reject) => {
      const id = ++counter;
      pending.set(id, {resolve, reject});
      worker.postMessage({id, request: {command, ...payload}});
    });
  }
  worker.onmessage = ({data}) => {
    const item = pending.get(data.id);
    if (!item) return;
    pending.delete(data.id);
    if (data.error) item.reject(new Error(data.error));
    else item.resolve(data.wave ? new Blob([data.wave], {type:"audio/wav"}) : data.value);
  };
  worker.onerror = () => {
    for (const item of pending.values()) item.reject(new Error("The music engine stopped. Download your song and reload the page."));
    pending.clear();
  };
  function url(blob) { const value = URL.createObjectURL(blob); blobs.add(value); return value; }
  function clearAudio() { for (const value of blobs) URL.revokeObjectURL(value); blobs.clear(); }
  function requireSong(path) {
    if (!current || current.path !== path) throw new Error("This song is no longer in the current session");
  }
  function downloadScore() {
    const link = document.querySelector("#downloadSong");
    if (!link || !current) return;
    if (link.href.startsWith("blob:")) URL.revokeObjectURL(link.href);
    link.href = URL.createObjectURL(new Blob([JSON.stringify(current.score, null, 2)], {type:"application/json"}));
    link.hidden = false;
  }
  async function inspected() {
    downloadScore();
    return {...await invoke("inspect", {score:current.score}), path:current.path, project:current.path,
      original_url:current.original, faithful_url:current.faithful};
  }
  async function request(endpoint, options = {}) {
    const route = endpoint.split("?")[0];
    const data = options.body ? JSON.parse(options.body) : {};
    if (route === "/api/projects") return {projects:current ? [{path:current.path, title:current.score.title, profile:current.score.profile, bpm:current.score.bpm}] : []};
    if (route === "/api/read") { requireSong(new URL(endpoint, location.href).searchParams.get("path")); return inspected(); }
    if (route === "/api/new") {
      clearAudio();
      current = {path:`session-${++counter}`, score:await invoke("new", data)};
      return inspected();
    }
    if (route === "/api/transcribe") {
      // Let Web Audio resample before entering WASM: a minute of 48/192 kHz
      // audio otherwise needs large FFT scratch buffers on a phone.
      const original = Uint8Array.from(atob(data.wav_base64), c => c.charCodeAt(0));
      const decoder = new OfflineAudioContext(1, 1, 16000);
      const decoded = await decoder.decodeAudioData(original.buffer.slice(0));
      if (decoded.duration > 60) throw new Error("A take can be at most one minute");
      const mono = new Float32Array(decoded.length);
      for (let channel = 0; channel < decoded.numberOfChannels; channel++) {
        const input = decoded.getChannelData(channel);
        for (let i = 0; i < input.length; i++) mono[i] += input[i] / decoded.numberOfChannels;
      }
      const normalized = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).split(",")[1]);
        reader.onerror = () => reject(new Error("Could not prepare recording"));
        reader.readAsDataURL(window.encodeWav(mono, 16000));
      });
      const result = await invoke("transcribe", {...data, wav_base64:normalized});
      let score = result.score;
      if (data.layer_on) {
        requireSong(data.layer_on);
        const previous = await invoke("inspect", {score:current.score});
        if (previous.score_sha256 !== data.expected_sha256) throw new Error("The song changed. Please try again.");
        score = await invoke("layer", {score:current.score, take:score, at_bar:data.layer_at_bar});
      }
      const continuous = await invoke("continuous", {wav_base64:normalized, transcription:result.transcription});
      clearAudio();
      current = {path:`session-${++counter}`, score, original:url(new Blob([original], {type:"audio/wav"})), faithful:url(continuous)};
      return {...await inspected(), result};
    }
    if (route === "/api/patch") {
      requireSong(data.path);
      const previous = await invoke("inspect", {score:current.score});
      if (previous.score_sha256 !== data.expected_sha256) throw new Error("The song changed. Please try again.");
      current.score = await invoke("patch", {score:current.score, operations:data.operations});
      return inspected();
    }
    if (route === "/api/render") {
      requireSong(data.path);
      const wave = await invoke("render", {score:current.score});
      if (current.mix) { URL.revokeObjectURL(current.mix); blobs.delete(current.mix); }
      current.mix = url(wave);
      const link = document.querySelector("#downloadWave");
      link.href = current.mix; link.hidden = false;
      return {audio_url:current.mix, title:current.score.title, seconds:current.score.length*60/current.score.bpm};
    }
    throw new Error("This action is unavailable in the browser edition");
  }
  window.ChipSession = {request};
  // A browser will dispose all memory when this tab closes; no storage API is used.
  window.addEventListener("pagehide", () => {
    clearAudio(); worker.terminate();
    current = null;
    for (const item of pending.values()) item.reject(new Error("Session ended"));
    pending.clear();
  }, {once:true});
  window.addEventListener("pageshow", event => {
    if (event.persisted) location.reload();
  });
})();
