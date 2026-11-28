import {describe, expect, it} from 'vitest';
import {outputProblem, outputSummary, YOUTUBE_URL} from './output';

const KEY = 'abcd-efgh-ijkl-mnop-wxyz';

describe('outputProblem', () => {
  it('accepts the YouTube RTMPS server and a key', () => {
    expect(outputProblem(YOUTUBE_URL, KEY)).toBeNull();
    expect(outputProblem(' rtmps://ingest.example.net:8443/app_1 ', ` ${KEY} `)).toBeNull();
  });
  it('rejects plain RTMP, IP literals, a key in the URL and bad keys without quoting the key', () => {
    for (const url of ['rtmp://a.rtmp.youtube.com/live2', 'rtmps://127.0.0.1/live2', 'rtmps://localhost/live2',
      `${YOUTUBE_URL}/${KEY}`, 'rtmps://a.rtmps.youtube.com', 'rtmps://a.rtmps.youtube.com:0/live2', 'rtmps://a.rtmps.youtube.com:70000/live2']) {
      expect(outputProblem(url, KEY), url).toMatch(/Server URL/);
    }
    for (const key of ['short', 'has/slash-key', 'with space key', 'x'.repeat(257)]) {
      const problem = outputProblem(YOUTUBE_URL, key);
      expect(problem).toMatch(/Stream key/);
      expect(problem).not.toContain(key);
    }
  });
});

describe('outputSummary', () => {
  it('shows only the last 4 characters to admins and only the state to operators', () => {
    expect(outputSummary(undefined)).toBe('Loading…');
    expect(outputSummary({configured: false})).toMatch(/^Not configured/);
    expect(outputSummary({configured: true})).toBe('Configured');
    expect(outputSummary({configured: true, url: YOUTUBE_URL, key_last4: 'wxyz', updated_at_unix: 1}))
      .toBe(`Configured: ${YOUTUBE_URL} · key ending wxyz`);
  });
});
