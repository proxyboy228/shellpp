// The terminal bell (UI 12.4 "Bell sound", off by default): a short tone made
// with Web Audio, so no audio asset is shipped. Bells within 200 ms of each
// other play once.
//
// Owner: S6.

type AudioContextConstructor = new () => AudioContext;

let context: AudioContext | null = null;
let lastBell = 0;

export function playBell(): void {
  const now = performance.now();
  if (now - lastBell < 200) {
    return;
  }
  lastBell = now;
  try {
    const Constructor = (window as unknown as { AudioContext?: AudioContextConstructor }).AudioContext;
    if (Constructor === undefined) {
      return;
    }
    context ??= new Constructor();
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.type = 'sine';
    oscillator.frequency.value = 880;
    gain.gain.setValueAtTime(0.08, context.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.12);
    oscillator.connect(gain);
    gain.connect(context.destination);
    oscillator.start();
    oscillator.stop(context.currentTime + 0.12);
  } catch {
    // No audio output: the bell is silent.
  }
}
