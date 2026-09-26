import * as vscode from 'vscode';
import type { AgentType } from './sessionProvider';

/** The mark shown for a session's agent. Claude's mark reads fine on any background; Copilot's near-black mark needs a separate white `dark` variant, via the `{ light, dark }` icon form. */
export function agentIconPath(extensionUri: vscode.Uri, agent: AgentType): vscode.Uri | { light: vscode.Uri; dark: vscode.Uri } {
  if (agent === 'claude') {
    return vscode.Uri.joinPath(extensionUri, 'resources', 'claude-mark.svg');
  }
  return {
    light: vscode.Uri.joinPath(extensionUri, 'resources', 'copilot-mark.svg'),
    dark: vscode.Uri.joinPath(extensionUri, 'resources', 'copilot-mark-dark.svg'),
  };
}
