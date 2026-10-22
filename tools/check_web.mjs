// Standalone WASM checks, without a browser, backend, or test dependencies.
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";

const bytes = await readFile(new URL("../docs/chip.wasm", import.meta.url));
const module = await WebAssembly.compile(bytes);
assert.deepEqual(WebAssembly.Module.imports(module), []);
const core = (await WebAssembly.instantiate(module, {})).exports;
function call(request) {
  const input = new TextEncoder().encode(JSON.stringify(request));
  const pointer = core.chip_alloc(input.length);
  assert.ok(pointer);
  new Uint8Array(core.memory.buffer, pointer, input.length).set(input);
  const packed = core.chip_run(pointer, input.length);
  core.chip_free(pointer, input.length);
  const address = Number(packed & 0xffffffffn), length = Number(packed >> 32n);
  const output = new Uint8Array(core.memory.buffer, address, length).slice();
  core.chip_free(address, length);
  if (!output[0]) throw new Error(new TextDecoder().decode(output.slice(1)));
  return output[0] === 1 ? JSON.parse(new TextDecoder().decode(output.slice(1))) : Buffer.from(output.slice(1));
}
const wav = Buffer.alloc(44 + 32000);
wav.write("RIFF"); wav.writeUInt32LE(wav.length-8, 4); wav.write("WAVEfmt ", 8);
wav.writeUInt32LE(16,16); wav.writeUInt16LE(1,20); wav.writeUInt16LE(1,22);
wav.writeUInt32LE(16000,24); wav.writeUInt32LE(32000,28);
wav.writeUInt16LE(2,32); wav.writeUInt16LE(16,34); wav.write("data",36); wav.writeUInt32LE(32000,40);
for (let i=0; i<16000; i++) wav.writeInt16LE(Math.round(9000*Math.sin(2*Math.PI*220*i/16000)),44+i*2);
const take = call({command:"transcribe", wav_base64:wav.toString("base64")});
assert.equal(take.score.patterns.melody.notes[0].note,57);
const layered = call({command:"layer", score:take.score, take:take.score, at_bar:1});
assert.equal(layered.tracks.length,2); assert.equal(layered.tracks[1].clips[0].at,4);
const audio = call({command:"render",score:layered});
assert.equal(audio.toString("ascii",0,4),"RIFF"); assert.ok(audio.length>1000);
const continuous = call({command:"continuous",wav_base64:wav.toString("base64"),transcription:take.transcription});
assert.equal(continuous.length,128044);
for (const profile of ["chip4","console8"]) call({command:"inspect",score:call({command:"new",profile})});
assert.throws(()=>call({command:"unsupported"}),/Unknown browser operation/);
assert.equal(core.chip_alloc(21*1024*1024),0);
console.log(`9 standalone WASM checks passed; ${bytes.length} bytes; zero host imports`);
