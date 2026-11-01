// Deterministic procedural art for the built-in wallpapers. No image assets are
// shipped: everything is generated from a seed so wallpapers are tiny, original,
// resolution-independent and identical on every run.

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export interface Star { x: number; y: number; r: number; o: number }

/** Stars in a 1600×900 viewBox. */
export function stars(seed: number, count: number, maxY = 900): Star[] {
  const rnd = mulberry32(seed);
  return Array.from({ length: count }, () => ({
    x: Math.round(rnd() * 1600),
    y: Math.round(rnd() * maxY),
    r: +(0.4 + rnd() ** 3 * 1.6).toFixed(2),
    o: +(0.25 + rnd() * 0.7).toFixed(2),
  }));
}

/** Midpoint-displacement ridge line closed to the bottom of a 1600×900 viewBox. */
export function ridge(seed: number, baseY: number, amplitude: number, roughness = 0.55, detail = 7): string {
  const rnd = mulberry32(seed);
  let pts: number[] = [baseY + (rnd() - 0.5) * amplitude, baseY + (rnd() - 0.5) * amplitude];
  let amp = amplitude;
  for (let i = 0; i < detail; i++) {
    const next: number[] = [];
    for (let j = 0; j < pts.length - 1; j++) {
      const a = pts[j]!;
      const b = pts[j + 1]!;
      next.push(a, (a + b) / 2 + (rnd() - 0.5) * amp);
    }
    next.push(pts[pts.length - 1]!);
    pts = next;
    amp *= roughness;
  }
  const step = 1600 / (pts.length - 1);
  let d = `M0 900 L0 ${pts[0]!.toFixed(1)}`;
  pts.forEach((y, i) => {
    d += ` L${(i * step).toFixed(1)} ${y.toFixed(1)}`;
  });
  return d + ' L1600 900 Z';
}
