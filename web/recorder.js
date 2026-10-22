"use strict";

class TakeRecorder {
  constructor(onLevel, onTime, onState, onTake) {
    this.onLevel = onLevel;
    this.onTime = onTime;
    this.onState = onState;
    this.onTake = onTake;
    this.context = null;
    this.stream = null;
    this.worklet = null;
    this.samples = [];
    this.startedAt = 0;
    this.timer = null;
    this.stopping = false;
    this.pendingStop = null;
  }

  async start() {
    if (this.context) throw new Error("Recording is already active.");
    if (!navigator.mediaDevices?.getUserMedia) throw new Error("Microphone unavailable: open the studio on localhost in a compatible browser.");
    this.onState("starting");
    this.samples = [];
    try {
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
      this.context = new AudioContext();
      this.contextSampleRate = this.context.sampleRate;
      await this.context.audioWorklet.addModule("/recorder.js");
      this.worklet = new AudioWorkletNode(this.context, "chip-recorder", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1] });
      this.worklet.port.onmessage = event => {
        if (event.data.type === "chunk") {
          this.samples.push(new Float32Array(event.data.samples));
          this.onLevel(event.data.level);
        } else if (event.data.type === "stopped" && this.pendingStop) {
          this.pendingStop();
        }
      };
      const source = this.context.createMediaStreamSource(this.stream);
      const silence = this.context.createGain();
      silence.gain.value = 0;
      source.connect(this.worklet).connect(silence).connect(this.context.destination);
      await this.context.resume();
      this.startedAt = Date.now();
      this.timer = setInterval(() => {
        const elapsed = (Date.now() - this.startedAt) / 1000;
        this.onTime(elapsed);
        if (elapsed >= 60) this.stop().then(blob => { if (blob) this.onTake(blob); });
      }, 100);
      this.onState("recording");
    } catch (error) {
      await this.cleanup();
      throw error;
    }
  }

  stop() {
    if (!this.worklet || this.stopping) return Promise.resolve(null);
    this.stopping = true;
    clearInterval(this.timer);
    return new Promise(resolve => {
      this.pendingStop = async () => {
        this.pendingStop = null;
        this.worklet.disconnect();
        await this.cleanup();
        this.onState("ready");
        resolve(this.toWav());
      };
      this.worklet.port.postMessage({ type: "stop" });
    });
  }

  async cleanup() {
    clearInterval(this.timer);
    if (this.stream) this.stream.getTracks().forEach(track => track.stop());
    this.stream = null;
    if (this.context) await this.context.close().catch(() => {});
    this.context = null;
    this.worklet = null;
    this.stopping = false;
  }

  toWav() {
    const length = this.samples.reduce((sum, block) => sum + block.length, 0);
    if (!length) throw new Error("The take is empty.");
    const pcm = new Float32Array(length);
    let offset = 0;
    for (const block of this.samples) { pcm.set(block, offset); offset += block.length; }
    const rate = this.contextSampleRate || 48000;
    return encodeWav(pcm, rate);
  }
}

function encodeWav(samples, sampleRate) {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const write = (offset, value) => [...value].forEach((ch, index) => view.setUint8(offset + index, ch.charCodeAt(0)));
  write(0, "RIFF"); view.setUint32(4, 36 + samples.length * 2, true); write(8, "WAVE");
  write(12, "fmt "); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  write(36, "data"); view.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++) {
    const value = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(44 + i * 2, value < 0 ? value * 32768 : value * 32767, true);
  }
  return new Blob([buffer], { type: "audio/wav" });
}

if (typeof AudioWorkletProcessor !== "undefined") {
  class ChipRecorderProcessor extends AudioWorkletProcessor {
    constructor() {
      super(); this.batch = new Float32Array(4096); this.index = 0; this.stopped = false;
      this.port.onmessage = event => {
        if (event.data.type === "stop") {
          if (this.index) this.flush();
          this.stopped = true; this.port.postMessage({ type: "stopped" });
        }
      };
    }
    flush() {
      const samples = this.batch.slice(0, this.index);
      let sum = 0; for (const sample of samples) sum += sample * sample;
      this.port.postMessage({ type: "chunk", samples, level: samples.length ? Math.sqrt(sum / samples.length) : 0 }, [samples.buffer]);
      this.index = 0;
    }
    process(inputs, outputs) {
      const input = inputs[0]?.[0];
      const output = outputs[0]?.[0];
      if (output) output.fill(0);
      if (!this.stopped && input) {
        for (const sample of input) {
          this.batch[this.index++] = sample;
          if (this.index === this.batch.length) this.flush();
        }
      }
      return !this.stopped;
    }
  }
  registerProcessor("chip-recorder", ChipRecorderProcessor);
}

if (typeof window !== "undefined") {
  window.TakeRecorder = TakeRecorder;
  window.encodeWav = encodeWav;
}
