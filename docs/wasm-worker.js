// Runs the same Rust transcription/synthesizer off the browser's UI thread.
const ready = fetch(new URL("./chip.wasm", import.meta.url))
  .then(response => { if (!response.ok) throw new Error("Could not load the music engine"); return response.arrayBuffer(); })
  .then(bytes => WebAssembly.instantiate(bytes, {}));
const encoder = new TextEncoder();
const decoder = new TextDecoder();
self.onmessage = async ({data: {id, request}}) => {
  try {
    const {instance: {exports: core}} = await ready;
    const input = encoder.encode(JSON.stringify(request));
    const pointer = core.chip_alloc(input.length);
    if (!pointer) throw new Error("Recording is too large for this browser");
    let packed;
    try {
      new Uint8Array(core.memory.buffer, pointer, input.length).set(input);
      packed = core.chip_run(pointer, input.length);
    } finally { core.chip_free(pointer, input.length); }
    const address = Number(packed & 0xffffffffn);
    const length = Number(packed >> 32n);
    const response = new Uint8Array(core.memory.buffer, address, length).slice();
    core.chip_free(address, length);
    if (!response[0]) throw new Error(decoder.decode(response.subarray(1)));
    if (response[0] === 1) self.postMessage({id, value: JSON.parse(decoder.decode(response.subarray(1)))});
    else {
      const bytes = response.slice(1);
      self.postMessage({id, wave: bytes.buffer}, [bytes.buffer]);
    }
  } catch (error) { self.postMessage({id, error: error.message || String(error)}); }
};
