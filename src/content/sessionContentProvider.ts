import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import * as readline from 'readline';
import { extractAssistantDisplayText, extractText, getClaudeProjectsDir, isDisplayableUserPrompt } from '../discovery/claudeStorage';
import { listCopilotTurns } from '../discovery/copilotStorage';
import { isInside } from '../discovery/pathUtils';

/** Custom URI scheme for the virtual, read-only documents that back the single reusable session tab. */
export const SESSION_SCHEME = 'session-deck';

/**
 * `session-deck:/claude/<projectDirNameEncoded>/<sessionId>.md` or `session-deck:/copilot/<sessionId>.md`
 * — see `viewTranscript` in extension.ts, the only place that builds these.
 */
export class SessionContentProvider implements vscode.TextDocumentContentProvider {
  private readonly _onDidChange = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this._onDidChange.event;

  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const segments = uri.path.replace(/^\//, '').split('/');
    const agent = segments[0];

    if (agent === 'copilot') {
      const sessionId = (segments[1] ?? '').replace(/\.md$/, '');
      return renderCopilotTranscript(sessionId);
    }

    const [, projectDirNameEncoded, fileName] = segments;
    const projectDirName = decodeURIComponent(projectDirNameEncoded ?? '');
    const sessionId = (fileName ?? '').replace(/\.md$/, '');

    const root = getClaudeProjectsDir();
    const filePath = path.join(root, projectDirName, `${sessionId}.jsonl`);

    // Defense in depth: decoded straight from the URI, so it must never resolve outside the projects root.
    if (!isInside(root, filePath)) {
      return '# Invalid session reference';
    }

    if (!fs.existsSync(filePath)) {
      return `# Session not found\n\n\`${filePath}\` no longer exists.`;
    }

    return renderClaudeTranscript(filePath, sessionId);
  }
}

/** Renders a Claude session's `.jsonl` transcript as readable markdown (user/assistant text turns only). */
async function renderClaudeTranscript(filePath: string, sessionId: string): Promise<string> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  const lines: string[] = [`# Session ${sessionId}`, ''];

  for await (const raw of rl) {
    if (!raw.trim()) {
      continue;
    }

    let record: unknown;
    try {
      record = JSON.parse(raw);
    } catch {
      continue;
    }
    if (!record || typeof record !== 'object') {
      continue;
    }

    const { type, message } = record as Record<string, unknown>;
    const msg = message as { role?: string; content?: unknown } | undefined;

    if (type === 'user' && msg?.role === 'user') {
      const text = extractText(msg.content).trim();
      if (text && isDisplayableUserPrompt(text)) {
        lines.push('## You', '', text, '');
      }
    } else if (type === 'assistant' && msg?.role === 'assistant') {
      // Thinking/tool_use blocks skipped for readability.
      const text = extractAssistantDisplayText(msg.content).trim();
      if (text) {
        lines.push('## Claude', '', text, '');
      }
    }
  }

  return lines.join('\n');
}

/** Renders a Copilot session's turns (from `~/.copilot/session-store.db`) the same way — one heading per side, in order. */
function renderCopilotTranscript(sessionId: string): string {
  const turns = listCopilotTurns(sessionId);
  if (turns.length === 0) {
    return `# Session not found\n\nNo turns found for \`${sessionId}\` in \`~/.copilot/session-store.db\`.`;
  }

  const lines: string[] = [`# Session ${sessionId}`, ''];
  for (const turn of turns) {
    if (turn.userMessage) {
      lines.push('## You', '', turn.userMessage, '');
    }
    if (turn.assistantResponse) {
      lines.push('## Copilot', '', turn.assistantResponse, '');
    }
  }
  return lines.join('\n');
}
