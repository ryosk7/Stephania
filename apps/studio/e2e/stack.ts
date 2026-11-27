// Starts the real engine and control plane for one test file.
import {spawn, type ChildProcess} from 'node:child_process';
import {mkdtempSync, openSync, readFileSync, rmSync} from 'node:fs';
import {createConnection, createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';

const ROOT = resolve(import.meta.dirname, '../../..');
export const TOKEN = 'studio-e2e-token-0123456789abcdef';

function freePort(): Promise<number> {
  return new Promise((ok, fail) => {
    const s = createServer();
    s.once('error', fail);
    s.listen(0, '127.0.0.1', () => {
      const port = (s.address() as {port: number}).port;
      s.close(() => ok(port));
    });
  });
}

async function waitFor(check: () => Promise<boolean>, what: string, log: string, ms = 30_000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error(`${what} did not start:\n${readFileSync(log, 'utf8')}`);
}

const connects = (opts: {path: string} | {port: number; host: string}) =>
  new Promise<boolean>(ok => {
    const c = createConnection(opts as never);
    c.once('connect', () => { c.destroy(); ok(true); });
    c.once('error', () => ok(false));
  });

export class Stack {
  readonly dir = mkdtempSync(join(tmpdir(), 't3box-studio-e2e-'));
  readonly socket = join(this.dir, 'engine.sock');
  port = 0;
  private engine?: ChildProcess;
  private control?: ChildProcess;

  get url() { return `http://localhost:${this.port}/`; }

  async startEngine() {
    const bin = process.env.T3BOX_ENGINE_BIN;
    if (!bin) throw new Error('T3BOX_ENGINE_BIN must point at a built t3box binary');
    const log = join(this.dir, 'engine.log');
    const fd = openSync(log, 'a');
    this.engine = spawn(bin, [], {env: {...process.env, T3BOX_ENGINE_SOCKET: this.socket, T3BOX_OUTPUT: ''}, stdio: ['ignore', fd, fd]});
    await waitFor(() => connects({path: this.socket}), 'engine', log);
  }

  async stopEngine() {
    const p = this.engine;
    this.engine = undefined;
    if (!p || p.exitCode !== null) return;
    await new Promise(r => { p.once('exit', r); p.kill('SIGTERM'); });
  }

  async start() {
    this.port = await freePort();
    await this.startEngine();
    const log = join(this.dir, 'control.log');
    const fd = openSync(log, 'a');
    this.control = spawn('bundle', ['exec', 'puma', '-C', 'config/puma.rb', 'config.ru'], {
      cwd: join(ROOT, 'services/control'),
      env: {...process.env, RACK_ENV: 'production', T3BOX_CONTROL_TOKEN: TOKEN, T3BOX_CONTROL_PORT: String(this.port),
        T3BOX_ENGINE_SOCKET: this.socket, T3BOX_CONTROL_DB: join(this.dir, 'control.db'), T3BOX_CONTROL_POLL_SECONDS: '0.2'},
      stdio: ['ignore', fd, fd],
    });
    await waitFor(() => connects({port: this.port, host: '127.0.0.1'}), 'control', log);
  }

  /** Stops everything; throws if the control plane ignores TERM for 10 s
   * (open event streams must not block shutdown). */
  async stop() {
    await this.stopEngine();
    const p = this.control;
    let hung = false;
    if (p && p.exitCode === null) {
      await new Promise<void>(r => {
        const timer = setTimeout(() => { hung = true; p.kill('SIGKILL'); }, 10_000);
        p.once('exit', () => { clearTimeout(timer); r(); });
        p.kill('SIGTERM');
      });
    }
    rmSync(this.dir, {recursive: true, force: true});
    if (hung) throw new Error('control plane did not stop within 10 s of SIGTERM');
  }
}
