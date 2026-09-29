import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as pty from 'node-pty';
import { Terminal } from '@xterm/headless';
import { SerializeAddon } from '@xterm/addon-serialize';
import { readDeckConfig } from '@session-deck/core';
import { detectScreenError, screenLines } from './screenStatus';

/** A running agent: its PTY plus a headless terminal mirroring its screen, so it can be drawn (preview) or repainted (attach) at any time. */
export interface LiveSession {
  pty: pty.IPty;
  term: Terminal;
  serializer: SerializeAddon;
  pid: number;
  exited: boolean;
  exitCode?: number;
  /** An error only visible on the screen (e.g. a failed sign-in), see `detectScreenError`. */
  screenError?: string;
  /** When the agent last printed anything: a prompt is typed only once its output has settled. */
  lastOutputAt: number;
}

export interface LiveSessionHandlers {
  onData(data: string): void;
  /** Whether the real terminal is currently showing this session (it then answers terminal queries itself). */
  isAttached(): boolean;
  onExit(exitCode: number): void;
}

export type AgentType = 'claude' | 'copilot';

const executables = new Map<AgentType, string>();

/** Forgets the resolved `tools.*.command` executables, so the next spawn re-reads the config file (used after editing it from the config popup). */
export function clearExecutableCache(): void {
  executables.clear();
}

/**
 * Pays `resolveExecutable`'s `where.exe` lookup (and, on an npm-only install, the shim-directory and
 * `package.json` reads) up front for every agent, so it's a one-time startup cost instead of landing on
 * whichever session gets attached first — that first `spawnAgent()` would otherwise block right as the
 * screen clears for the mirrored PTY snapshot, turning into a visible stall/flash.
 */
export function warmExecutables(): void {
  for (const agent of Object.keys(NPM_PACKAGES) as AgentType[]) {
    resolveExecutable(agent);
  }
}

/** The npm package that installs each agent's CLI, used to find its real binary when it was only installed via `npm install -g` (no system installer put a `.exe` on PATH). */
const NPM_PACKAGES: Record<AgentType, string> = {
  claude: '@anthropic-ai/claude-code',
  copilot: '@github/copilot',
};

/**
 * Given the directory of an agent's npm-global shim (`<dir>/<agent>.cmd`), resolves the real binary its
 * package ships, from that package's own `package.json#bin` entry — not a path assumed or hardcoded, so
 * it holds however npm laid the install out. Returns the binary's absolute path only when it's a native
 * `.exe` that actually exists (Claude's `bin/claude.exe`), so ConPTY can run it directly instead of the
 * shim (which would otherwise add a `cmd.exe` hop). Returns undefined when the package's bin is a JS
 * entry point instead (e.g. Copilot's loader script), which has no equivalent shortcut.
 */
export function resolveNpmGlobalExecutable(agent: AgentType, shimDir: string): string | undefined {
  const pkgDir = path.join(shimDir, 'node_modules', NPM_PACKAGES[agent]);
  let pkg: { bin?: Record<string, string> | string };
  try {
    pkg = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8')) as { bin?: Record<string, string> | string };
  } catch {
    return undefined;
  }
  const binEntry = typeof pkg.bin === 'string' ? pkg.bin : pkg.bin?.[agent];
  if (!binEntry || !binEntry.endsWith('.exe')) {
    return undefined;
  }
  const exePath = path.join(pkgDir, binEntry);
  return fs.existsSync(exePath) ? exePath : undefined;
}

/**
 * The agent's real executable: the `tools.<agent>.command` override from `~/.session-deck/config.json`
 * if set (used as-is, whether a bare name or a full path), otherwise `where.exe` on Windows (so ConPTY
 * runs the .exe rather than an npm .cmd shim) — falling back, when no bare `.exe` is on PATH (an
 * npm-only install, with no system installer's own `.exe`), to whatever real binary the npm-global
 * install's own package ships, so the agent starts the same way regardless of how it was installed —
 * otherwise the bare agent name.
 */
function resolveExecutable(agent: AgentType): string {
  let exe = executables.get(agent);
  if (!exe) {
    const configured = readDeckConfig().tools[agent].command;
    if (configured) {
      exe = configured;
    } else if (process.platform === 'win32') {
      try {
        exe = execFileSync('where.exe', [`${agent}.exe`], { encoding: 'utf8' }).split(/\r?\n/)[0].trim();
      } catch {
        let shimDir: string | undefined;
        try {
          shimDir = path.dirname(execFileSync('where.exe', [`${agent}.cmd`], { encoding: 'utf8' }).split(/\r?\n/)[0].trim());
        } catch {
          shimDir = undefined;
        }
        exe = (shimDir && resolveNpmGlobalExecutable(agent, shimDir)) || `${agent}.exe`;
      }
    } else {
      exe = agent;
    }
    executables.set(agent, exe);
  }
  return exe;
}

/**
 * Command-line arguments to resume `sessionId`, or to start a new session (Copilot takes a pre-assigned
 * id, Claude assigns its own), followed by any extra `tools.<agent>.args` from the config file.
 */
function agentArgs(agent: AgentType, sessionId: string | null, isNew: boolean): string[] {
  const base =
    agent === 'copilot'
      ? sessionId
        ? [isNew ? `--session-id=${sessionId}` : `--resume=${sessionId}`]
        : []
      : sessionId && !isNew
        ? ['--resume', sessionId]
        : [];
  return [...base, ...(readDeckConfig().tools[agent].args ?? [])];
}

/** Vars a parent Claude Code process sets for its children — inherited, they make the agent think it's a sub-session (e.g. transcript saving off) when sdeck is run from inside Claude Code. */
const INHERITED_CLAUDE_VARS = [
  'CLAUDECODE',
  'CLAUDE_PID',
  'CLAUDE_EFFORT',
  'CLAUDE_CODE_CHILD_SESSION',
  'CLAUDE_CODE_SESSION_ID',
  'CLAUDE_CODE_SESSION_ATTENDED',
  'CLAUDE_CODE_MESSAGING_SOCKET',
  'CLAUDE_CODE_MESSAGING_TOKEN',
  'CLAUDE_CODE_ENTRYPOINT',
  'CLAUDE_CODE_EXECPATH',
  'CLAUDE_CODE_SSE_PORT',
];

function agentEnv(): Record<string, string> {
  const env: Record<string, string> = { COLORTERM: 'truecolor' };
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined && !INHERITED_CLAUDE_VARS.includes(key) && key !== 'COLORTERM') {
      env[key] = value;
    }
  }
  return env;
}

/**
 * Starts the agent in a background PTY of the given size: resuming `sessionId`, or starting a new
 * session when `isNew` (with `sessionId` as its pre-assigned id, for Copilot).
 */
export function spawnAgent(
  agent: AgentType,
  sessionId: string | null,
  isNew: boolean,
  cwd: string,
  cols: number,
  rows: number,
  handlers: LiveSessionHandlers
): LiveSession {
  const proc = pty.spawn(resolveExecutable(agent), agentArgs(agent, sessionId, isNew), {
    name: 'xterm-256color',
    cols,
    rows,
    cwd,
    env: agentEnv(),
  });
  // cursorStyle: neither agent ever negotiates a shape (DECSCUSR) itself, so this is the one place
  // that decides it — matching it here is what keeps the preview's drawn-in cursor (view.ts/ansi.ts
  // renderTerm) looking like the real cursor an attached terminal shows.
  const term = new Terminal({ cols, rows, scrollback: 2000, allowProposedApi: true, cursorStyle: 'underline' });
  const serializer = new SerializeAddon();
  term.loadAddon(serializer);
  const live: LiveSession = { pty: proc, term, serializer, pid: proc.pid, exited: false, lastOutputAt: Date.now() };

  // At most twice a second, after output settles into the mirror.
  let screenCheck: NodeJS.Timeout | undefined;
  const checkScreen = () => {
    screenCheck = undefined;
    live.screenError = detectScreenError(screenLines(term));
  };
  proc.onData((data) => {
    term.write(data);
    live.lastOutputAt = Date.now();
    screenCheck ??= setTimeout(checkScreen, 500);
    handlers.onData(data);
  });
  // Replies to terminal queries (cursor position, device attributes...). While attached the real
  // terminal answers instead, so forwarding both would double-reply.
  term.onData((data) => {
    if (!handlers.isAttached() && !live.exited) {
      proc.write(data);
    }
  });
  proc.onExit(({ exitCode }) => {
    live.exited = true;
    live.exitCode = exitCode;
    handlers.onExit(exitCode);
  });
  return live;
}

export function resizeLive(live: LiveSession | undefined, cols: number, rows: number): void {
  if (!live || live.exited || (live.term.cols === cols && live.term.rows === rows)) {
    return;
  }
  try {
    live.pty.resize(cols, rows);
  } catch {
    // raced with exit
  }
  live.term.resize(cols, rows);
}

export function disposeLive(live: LiveSession): void {
  if (!live.exited) {
    try {
      live.pty.kill();
    } catch {
      // already gone
    }
  }
  live.term.dispose();
}

/** Types a line into the agent and submits it: text and Enter as separate writes, so the input box doesn't treat the Enter as part of a paste. */
export function typeLine(live: LiveSession, text: string): void {
  live.pty.write(text);
  setTimeout(() => !live.exited && live.pty.write('\r'), 150);
}
