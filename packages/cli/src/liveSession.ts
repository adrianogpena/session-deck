import { execFileSync } from 'child_process';
import * as pty from 'node-pty';
import { Terminal } from '@xterm/headless';
import { SerializeAddon } from '@xterm/addon-serialize';

/** A running agent: its PTY plus a headless terminal mirroring its screen, so it can be drawn (preview) or repainted (attach) at any time. */
export interface LiveSession {
  pty: pty.IPty;
  term: Terminal;
  serializer: SerializeAddon;
  pid: number;
  exited: boolean;
  exitCode?: number;
}

export interface LiveSessionHandlers {
  onData(data: string): void;
  /** Whether the real terminal is currently showing this session (it then answers terminal queries itself). */
  isAttached(): boolean;
  onExit(exitCode: number): void;
}

let claudeExe: string | undefined;

function resolveClaudeExe(): string {
  if (claudeExe) {
    return claudeExe;
  }
  claudeExe = 'claude';
  if (process.platform === 'win32') {
    try {
      claudeExe = execFileSync('where.exe', ['claude.exe'], { encoding: 'utf8' }).split(/\r?\n/)[0].trim();
    } catch {
      claudeExe = 'claude.exe';
    }
  }
  return claudeExe;
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

/** Starts `claude` (resuming `sessionId` when given) in a background PTY of the given size. */
export function spawnClaude(sessionId: string | null, cwd: string, cols: number, rows: number, handlers: LiveSessionHandlers): LiveSession {
  const proc = pty.spawn(resolveClaudeExe(), sessionId ? ['--resume', sessionId] : [], {
    name: 'xterm-256color',
    cols,
    rows,
    cwd,
    env: agentEnv(),
  });
  const term = new Terminal({ cols, rows, scrollback: 2000, allowProposedApi: true });
  const serializer = new SerializeAddon();
  term.loadAddon(serializer);
  const live: LiveSession = { pty: proc, term, serializer, pid: proc.pid, exited: false };

  proc.onData((data) => {
    term.write(data);
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
