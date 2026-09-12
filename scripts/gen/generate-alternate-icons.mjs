#!/usr/bin/env node
// Builds every ALTERNATE app icon as an Icon Composer document, in the same
// Liquid Glass language as the primary (Moshpit/Resources/AppIcon.icon): white glass
// figures on a coloured fill, a darker fill for the Dark appearance, and the
// system deriving Clear and Tinted on its own. Each alternate is a DIFFERENT
// figure — the picker offers real alternatives, not recolours of one mark —
// and each carries one fill colour, so the colour range survives.
//
// Output (all checked in):
//   Moshpit/Resources/AlternateIcons/AppIcon-<Tag>.icon/   icon.json + Assets/*.png
//   Moshpit/Resources/AlternateIcons/Previews/IconPreview-<Tag>@3x.png
//     the picker's thumbnails, rendered by ictool exactly the way SpringBoard
//     renders the icon on iOS 26 (also one for the primary, "Moshpit").
//
// Xcode compiles the documents because project.yml lists their names in
// ASSETCATALOG_COMPILER_ALTERNATE_APPICON_NAMES; actool then writes the
// CFBundleAlternateIcons entries into Info.plist and flattens iOS 18/19
// fallbacks itself. No loose PNGs, no hand-written plist blocks.
//
// Usage: node scripts/gen/generate-alternate-icons.mjs [--sheet /tmp/sheet.png]
// Needs: rsvg-convert (brew install librsvg) and Xcode 26's Icon Composer.
import { execFileSync } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const OUT = join(ROOT, 'Moshpit/Resources/AlternateIcons');
const PRIMARY = join(ROOT, 'Moshpit/Resources/AppIcon.icon');
const ICTOOL = '/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool';
const W = '#FFFFFF';

// ── SVG helpers (1024 canvas, the icon's own coordinate space) ─────────────
const svg = (body) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">${body}</svg>`;
const rr = (x, y, w, h, r, fill = W, extra = '') =>
  `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}" fill="${fill}"${extra}/>`;
const mono = (text, size, x, y, fill = W, extra = '') =>
  `<text x="${x}" y="${y}" font-family="Menlo, Menlo-Bold" font-weight="bold" font-size="${size}" fill="${fill}" text-anchor="middle" dominant-baseline="central"${extra}>${text}</text>`;
/// A white figure with black shapes punched out of it — the primary's cursor
/// hole, generalised. The fill shows through the holes.
const punched = (figure, holes) =>
  `<mask id="m"><rect width="1024" height="1024" fill="#fff"/>${holes}</mask><g mask="url(#m)">${figure}</g>`;
const stroke = (w, fill = 'none') => ` fill="${fill}" stroke="${W}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"`;

// ── Fills ──────────────────────────────────────────────────────────────────
const srgb = (hex) => {
  const n = parseInt(hex.slice(1), 16);
  const c = (v) => (v / 255).toFixed(5);
  return `extended-srgb:${c(n >> 16)},${c((n >> 8) & 255)},${c(n & 255)},1.00000`;
};
const gradient = (top, bottom) => ({ 'linear-gradient': [srgb(top), srgb(bottom)] });
const seed = (hex) => ({ 'automatic-gradient': srgb(hex) });

// ── The gallery ────────────────────────────────────────────────────────────
// 24-grid → 1024 mapping the shipped 1.0.x icons were drawn in (7.5 % inset).
const U = (1024 * 0.85) / 24, O = 1024 * 0.075;
const g = (v) => O + v * U;
const grow = (body, k = 1.2, cx = 512, cy = 555) => `<g transform="translate(${cx} ${cy}) scale(${k}) translate(${-cx} ${-cy})">${body}</g>`;

const icons = [
  {
    // The crowd-surf mark that was primary from 1.0.0 to 1.0.2; the icon a
    // lot of people bought the app with, so it stays. Same geometry, now
    // three grounded glass blocks and one glass surfer over them.
    tag: 'Pit', light: gradient('#3A3A78', '#1C1C44'), dark: gradient('#262654', '#0E0E28'),
    // Scaled up a fifth from the shipped geometry: the 1.0.x tile left a
    // wide margin that reads as small next to the other figures.
    layers: [
      { name: 'surfer', body: grow(rr(g(8.4), g(8.0), 6.8 * U, 4.0 * U, 1.1 * U, W, ` transform="rotate(-12 ${g(11.8)} ${g(10)})"`)) },
      { name: 'crowd', opacity: 0.7, body: grow([3.6, 9.8, 16.0].map((x) => rr(g(x), g(15.6), 4.4 * U, 2.8 * U, 0.8 * U)).join('')) },
    ],
  },
  {
    // The Dynamic Island with an agent's status in it — the thing this app
    // does that no other terminal does. A status chip: dot, then a line of
    // text, both cut through the glass so the black shows.
    tag: 'Island', light: gradient('#2C2C31', '#0F0F12'), dark: gradient('#1C1C1F', '#050506'),
    layers: [
      { name: 'island', body: punched(rr(172, 392, 680, 240, 120), `<circle cx="322" cy="512" r="44" fill="#000"/>${rr(412, 480, 328, 64, 32, '#000')}`) },
    ],
  },
  {
    // ":wq" — the one vim command everybody knows. Type as glass.
    tag: 'Wq', light: gradient('#3FBF86', '#1F7E52'), dark: gradient('#2B8F62', '#114A30'),
    layers: [{ name: 'wq', body: mono(':wq', 340, 512, 522) }],
  },
  {
    // "⌃b" on a keycap — the tmux prefix, muscle memory made visible. The
    // legend is punched through the cap; a second slab under it gives the key
    // its side.
    tag: 'Prefix', light: gradient('#666B80', '#3B3F50'), dark: gradient('#434756', '#1F2129'),
    layers: [
      { name: 'cap', body: punched(rr(202, 272, 620, 450, 104), mono('⌃b', 290, 512, 505, '#000', ' stroke="#000" stroke-width="16"')) },
      { name: 'base', opacity: 0.45, body: rr(202, 306, 620, 450, 104) },
    ],
  },
  {
    // A house whose door is the cursor — there's no place like 127.0.0.1.
    // Daylight sky, the one bright tile in the set.
    tag: 'Localhost', light: gradient('#62ACF4', '#3A80D2'), dark: gradient('#3D74BA', '#1F4B82'),
    layers: [
      {
        name: 'house',
        body: punched(
          `<path d="M512 232 L826 480 L760 480 L760 780 L264 780 L264 480 L198 480 Z"${stroke(56, W)}/>`,
          rr(448, 566, 128, 260, 30, '#000')),
      },
    ],
  },
  {
    // A prompt waiting for you: the chevron and the block. Amber, the colour
    // the app uses for "an agent needs you" — and of the terminals before it.
    tag: 'Prompt', light: gradient('#DDA043', '#A96F1E'), dark: gradient('#A87626', '#6A4510'),
    layers: [
      { name: 'chevron', body: `<polyline points="330,352 500,512 330,672"${stroke(96)}/>` },
      { name: 'block', body: rr(586, 400, 176, 224, 40) },
    ],
  },
  {
    // The cursor block alone, oversized — the terminal reduced to its
    // heartbeat. On the primary's violet: this is the block from its hole.
    tag: 'Cursor', light: seed('#6C6BEF'), dark: gradient('#4D4A9E', '#29265C'),
    layers: [{ name: 'cursor', body: rr(262, 362, 500, 300, 64) }],
  },
  {
    // "❯))" — a prompt hailing two arcs out: mosh's roaming, the session that
    // follows you.
    tag: 'Hail', light: gradient('#3ABBA8', '#19897B'), dark: gradient('#238C7D', '#0C4F47'),
    layers: [
      { name: 'chevron', body: `<polyline points="280,332 480,512 280,692"${stroke(88)}/>` },
      { name: 'arcs', body: `<path d="M ${410 + 210 * Math.SQRT1_2} ${512 - 210 * Math.SQRT1_2} A 210 210 0 0 1 ${410 + 210 * Math.SQRT1_2} ${512 + 210 * Math.SQRT1_2}"${stroke(78)}/>` },
      { name: 'arc-far', opacity: 0.55, body: `<path d="M ${410 + 340 * Math.SQRT1_2} ${512 - 340 * Math.SQRT1_2} A 340 340 0 0 1 ${410 + 340 * Math.SQRT1_2} ${512 + 340 * Math.SQRT1_2}"${stroke(78)}/>` },
    ],
  },
];

// ── Build ──────────────────────────────────────────────────────────────────
const render = (markup, path) =>
  execFileSync('rsvg-convert', ['-w', '1024', '-h', '1024', '-b', 'transparent', '-o', path], { input: svg(markup) });

for (const icon of icons) {
  const doc = join(OUT, `AppIcon-${icon.tag}.icon`);
  rmSync(doc, { recursive: true, force: true });
  mkdirSync(join(doc, 'Assets'), { recursive: true });
  for (const layer of icon.layers) render(layer.body, join(doc, 'Assets', `${layer.name}.png`));
  const json = {
    'fill-specializations': [
      { value: icon.light },
      { appearance: 'dark', value: icon.dark },
    ],
    groups: [{
      layers: icon.layers.map((l) => ({ glass: true, 'image-name': `${l.name}.png`, name: l.name, ...(l.opacity ? { opacity: l.opacity } : {}) })),
      lighting: 'individual',
      shadow: { kind: 'neutral', opacity: 0.5 },
      specular: true,
      translucency: { enabled: true, value: 0.5 },
    }],
    'supported-platforms': { squares: 'shared' },
  };
  writeFileSync(join(doc, 'icon.json'), JSON.stringify(json, null, 2) + '\n');
  console.log('icon', icon.tag, icon.layers.map((l) => l.name).join('+'));
}

// ── Previews: what the picker shows, rendered like the home screen ─────────
const previews = join(OUT, 'Previews');
mkdirSync(previews, { recursive: true });
const preview = (doc, tag, rendition = 'Default', file = join(previews, `IconPreview-${tag}@3x.png`)) =>
  execFileSync(ICTOOL, [doc, '--export-image', '--output-file', file, '--platform', 'iOS', '--rendition', rendition, '--width', '80', '--height', '80', '--scale', '3']);
preview(PRIMARY, 'Moshpit');
for (const icon of icons) preview(join(OUT, `AppIcon-${icon.tag}.icon`), icon.tag);
console.log('previews', ['Moshpit', ...icons.map((i) => i.tag)].join(' '));

// ── Optional review sheet: every icon, Default and Dark ────────────────────
const sheetArg = process.argv.indexOf('--sheet');
if (sheetArg > 0) {
  const tmp = join(process.env.TMPDIR ?? '/tmp', 'moshpit-icon-sheet');
  rmSync(tmp, { recursive: true, force: true }); mkdirSync(tmp, { recursive: true });
  const docs = [['Moshpit', PRIMARY], ...icons.map((i) => [i.tag, join(OUT, `AppIcon-${i.tag}.icon`)])];
  for (const [tag, doc] of docs) for (const r of ['Default', 'Dark'])
    execFileSync(ICTOOL, [doc, '--export-image', '--output-file', join(tmp, `${tag}-${r}.png`), '--platform', 'iOS', '--rendition', r, '--width', '256', '--height', '256', '--scale', '1']);
  const py = `
from PIL import Image, ImageDraw
import sys
tmp, out = sys.argv[1], sys.argv[2]
tags = sys.argv[3].split(',')
T = 300
sheet = Image.new('RGBA', (T*len(tags), 2*T+40), '#0b0b0d')
d = ImageDraw.Draw(sheet)
for i, tag in enumerate(tags):
    for j, (r, bg) in enumerate((('Default', '#f2f2f7'), ('Dark', '#000000'))):
        tile = Image.new('RGBA', (T, T), bg)
        im = Image.open(f'{tmp}/{tag}-{r}.png').convert('RGBA').resize((256, 256))
        tile.alpha_composite(im, (22, 22)); sheet.paste(tile, (i*T, j*T))
    d.text((i*T+22, 2*T+12), tag, fill='#aaa')
sheet.save(out)
`;
  execFileSync('python3', ['-c', py, tmp, process.argv[sheetArg + 1], docs.map((d) => d[0]).join(',')]);
  console.log('sheet', process.argv[sheetArg + 1]);
}
