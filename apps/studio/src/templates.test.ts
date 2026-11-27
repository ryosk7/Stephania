import {describe, expect, it} from 'vitest';
import {BUILTIN, checkDeveloperSvg, initialVars, reference, substitute, svgSize, templateSvg} from './templates';

describe('templates', () => {
  it('substitutes variables with XML escaping', () => {
    expect(substitute('<text>{{speaker.name}} / {{ missing }}</text>', {'speaker.name': '<b>&"内田"</b>'}))
      .toBe('<text>&lt;b&gt;&amp;&quot;内田&quot;&lt;/b&gt; / </text>');
  });
  it('renders the built-in lower third with escaped values', () => {
    const def = BUILTIN.find(t => t.id === 'lower-third')!;
    const svg = templateSvg(def, {...initialVars(def), 'speaker.name': 'A<script>'});
    expect(svg).toContain('A&lt;script&gt;');
    expect(svg).not.toContain('<script');
    expect(checkDeveloperSvg(svg)).toBeNull();
    expect(svgSize(svg)).toEqual({width: 1000, height: 160});
  });
  it('rejects developer SVG that could escape the image sandbox', () => {
    const ok = '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect fill="url(#g)" width="10" height="10"/></svg>';
    expect(checkDeveloperSvg(ok)).toBeNull();
    for (const bad of [
      '<svg><script>alert(1)</script></svg>',
      '<svg><rect onload="x()"/></svg>',
      '<svg><image href="https://evil.example/x.png"/></svg>',
      '<svg><a xlink:href="javascript:alert(1)"/></svg>',
      '<svg><style>rect{fill:url(https://evil.example/)}</style></svg>',
      '<!DOCTYPE svg [<!ENTITY x "y">]><svg></svg>',
      'not svg',
      '<svg>' + 'x'.repeat(20001) + '</svg>',
    ]) expect(checkDeveloperSvg(bad), bad.slice(0, 40)).not.toBeNull();
  });
  it('stores only declared variables, bounded', () => {
    const def = BUILTIN.find(t => t.id === 'title')!;
    expect(reference(def, {title: 'x'.repeat(300), extra: 'y'})).toEqual({id: 'title', vars: {title: 'x'.repeat(200), color: ''}});
  });
  it('reads SVG size in either attribute order', () => {
    expect(svgSize('<svg height="20" width="30">')).toEqual({width: 30, height: 20});
    expect(svgSize('<svg width="3000" height="20">')).toBeNull();
  });
});
