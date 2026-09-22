import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import * as readline from 'readline';
import { extractAssistantDisplayText, extractText, getClaudeProjectsDir, isDisplayableUserPrompt } from './claudeStorage';
import { isInside } from './pathUtils';

/** Custom URI scheme for the virtual, read-only documents that back the single reusable session tab. */
export const SESSION_SCHEME = 'session-deck';

export class SessionContentProvider implements vscode.TextDocumentContentProvider {
  private readonly _onDidChange = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this._onDidChange.event;

  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const [projectDirNameEncoded, fileName] = uri.path.replace(/^\//, '').split('/');
    const projectDirName = decodeURIComponent(projectDirNameEncoded ?? '');
    const sessionId = (fileName ?? '').replace(/\.md$/, '');

    const root = getClaudeProjectsDir();
    const filePath = path.join(root, projectDirName, `${sessionId}.jsonl`);

    // Defense in depth: projectDirName/sessionId are decoded straight from the URI, so a
    // "../" (or similar) segment must never resolve outside ~/.claude/projects, even though
    // every URI this extension itself builds (viewTranscript) only ever uses real directory/file
    // names already enumerated from that same tree.
    if (!isInside(root, filePath)) {
      return '# Invalid session reference';
    }

    if (!fs.existsSync(filePath)) {
      return `# Session not found\n\n\`${filePath}\` no longer exists.`;
    }

    return renderTranscript(filePath, sessionId);
  }
}

/** Renders a session's .jsonl transcript as readable markdown (user/assistant text turns only). */
async function renderTranscript(filePath: string, sessionId: string): Promise<string> {
  const rl = readline.createInterface({ input: fs.createReadStream(filePath), crlfDelay: Infinity });
  const lines: string[] = [`# Session ${sessionId}`, ''];

  for await (const raw of rl) {
    if (!raw.trim()) {
      continue;
    }

    let record: any;
    try {
      record = JSON.parse(raw);
    } catch {
      continue;
    }

    if (record.type === 'user' && record.message?.role === 'user') {
      const text = extractText(record.message.content).trim();
      if (text && isDisplayableUserPrompt(text)) {
        lines.push('## You', '', text, '');
      }
    } else if (record.type === 'assistant' && record.message?.role === 'assistant') {
      // Thinking/tool_use blocks are skipped for readability; consider a
      // collapsible representation of tool calls once this moves past skeleton stage.
      const text = extractAssistantDisplayText(record.message.content).trim();
      if (text) {
        lines.push('## Claude', '', text, '');
      }
    }
  }

  return lines.join('\n');
}
