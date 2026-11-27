// Overlay templates rendered in the browser into PNG assets (docs/SCENE.md
// "Templates"). The engine has no fonts and never sees markup: a template
// is turned into SVG here, drawn through an <img> (no scripts, no network,
// no access to the page) onto a canvas, and uploaded as an image.
import type {Template} from './editor';

export type TemplateDef = {
  id: string;
  name: string;
  width: number;
  height: number;
  vars: {key: string; label: string; initial: string}[];
  /** Developer templates carry their own SVG with {{var}} placeholders. */
  svg?: string;
};

export const BUILTIN: TemplateDef[] = [
  {id: 'lower-third', name: 'Lower third', width: 1000, height: 160, vars: [
    {key: 'speaker.name', label: 'Speaker name', initial: 'Speaker name'},
    {key: 'speaker.title', label: 'Title', initial: 'Affiliation and title'},
    {key: 'event.name', label: 'Event name', initial: 'Event name'},
  ]},
  {id: 'title', name: 'Title', width: 1600, height: 200, vars: [
    {key: 'title', label: 'Text', initial: 'Title'},
    {key: 'color', label: 'Text colour', initial: '#ffffff'},
  ]},
];

export const MAX_SVG_CHARS = 20000;

const xml = (s: string) => s.replace(/[&<>"']/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;'}[c]!));
const color = (s: string | undefined, fallback: string) => (s && /^#[0-9a-fA-F]{6}$/.test(s) ? s : fallback);

/** Replace {{key}} with the XML-escaped value (unknown keys become empty). */
export function substitute(svg: string, vars: Record<string, string>): string {
  return svg.replace(/\{\{\s*([a-z][a-z0-9_.]{0,31})\s*\}\}/g, (_, k: string) => xml(vars[k] ?? ''));
}

/**
 * Reject developer SVG that could reach outside the image sandbox. Rendering
 * through <img> already blocks scripts and external loads; this keeps the
 * stored template honest and the failure visible.
 */
export function checkDeveloperSvg(svg: string): string | null {
  if (svg.length > MAX_SVG_CHARS) return `SVG must be at most ${MAX_SVG_CHARS} characters`;
  if (!/^\s*<svg[\s>]/i.test(svg) || !/<\/svg>\s*$/i.test(svg)) return 'SVG must start with <svg> and end with </svg>';
  if (/<script|<(iframe|object|embed)|\son[a-z]+\s*=|javascript:|<!ENTITY|<!DOCTYPE/i.test(svg)) return 'Scripts and embedded content are not allowed';
  if (/(href|src)\s*=\s*["']\s*(?!#|data:image\/)/i.test(svg)) return 'External references are not allowed (only #id and data:image)';
  if (/url\(\s*["']?\s*(?!#|data:image\/)/i.test(svg)) return 'External CSS url() is not allowed';
  return null;
}

/** SVG markup for a template and its variables. */
export function templateSvg(def: TemplateDef, vars: Record<string, string>): string {
  const v = (k: string) => xml(vars[k] ?? '');
  const font = `font-family="system-ui,'Hiragino Sans','Noto Sans JP',sans-serif"`;
  if (def.svg) return substitute(def.svg, vars);
  if (def.id === 'lower-third') {
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${def.width}" height="${def.height}">` +
      `<rect x="0" y="20" width="${def.width}" height="120" rx="10" fill="#0b1220" fill-opacity="0.86"/>` +
      `<rect x="0" y="20" width="14" height="120" rx="4" fill="#3b82f6"/>` +
      `<text x="40" y="82" ${font} font-size="50" font-weight="700" fill="#ffffff">${v('speaker.name')}</text>` +
      `<text x="42" y="124" ${font} font-size="28" fill="#cbd5e1">${v('speaker.title')}</text>` +
      `<text x="${def.width - 24}" y="124" ${font} font-size="24" fill="#94a3b8" text-anchor="end">${v('event.name')}</text>` +
      `</svg>`;
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${def.width}" height="${def.height}">` +
    `<text x="${def.width / 2}" y="${def.height / 2}" ${font} font-size="96" font-weight="700" fill="${color(vars.color, '#ffffff')}"` +
    ` text-anchor="middle" dominant-baseline="central" stroke="#000000" stroke-opacity="0.5" stroke-width="3" paint-order="stroke">${v('title')}</text></svg>`;
}

export function templateFor(id: string, custom: TemplateDef[] = []): TemplateDef | undefined {
  return [...BUILTIN, ...custom].find(t => t.id === id);
}

export function initialVars(def: TemplateDef): Record<string, string> {
  return Object.fromEntries(def.vars.map(v => [v.key, v.initial]));
}

/** Template reference stored on the image layer. */
export function reference(def: TemplateDef, vars: Record<string, string>): Template {
  return {id: def.id, vars: Object.fromEntries(def.vars.map(v => [v.key, (vars[v.key] ?? '').slice(0, 200)]))};
}

/** Render SVG to a PNG blob in the browser (no scripts, no network). */
export async function renderPng(svg: string, width: number, height: number): Promise<Blob> {
  const url = URL.createObjectURL(new Blob([svg], {type: 'image/svg+xml'}));
  try {
    const img = new Image(width, height);
    await new Promise<void>((ok, fail) => { img.onload = () => ok(); img.onerror = () => fail(new Error('Cannot render the SVG')); img.src = url; });
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    canvas.getContext('2d')!.drawImage(img, 0, 0, width, height);
    return await new Promise<Blob>((ok, fail) => canvas.toBlob(b => (b ? ok(b) : fail(new Error('Cannot convert to PNG'))), 'image/png'));
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Width/height from an SVG root, for developer templates. */
export function svgSize(svg: string): {width: number; height: number} | null {
  const m = /^\s*<svg[^>]*\swidth="(\d{1,4})"[^>]*\sheight="(\d{1,4})"/i.exec(svg) ?? /^\s*<svg[^>]*\sheight="(\d{1,4})"[^>]*\swidth="(\d{1,4})"/i.exec(svg);
  if (!m) return null;
  const [w, h] = /width="\d+"[^>]*height=/i.test(m[0]) ? [Number(m[1]), Number(m[2])] : [Number(m[2]), Number(m[1])];
  return w >= 1 && h >= 1 && w <= 1920 && h <= 1080 ? {width: w, height: h} : null;
}
