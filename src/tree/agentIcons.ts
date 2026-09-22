import * as vscode from 'vscode';
import type { AgentType } from './sessionProvider';

/**
 * The mark shown for a session's agent, both in the tree and on its terminal tab. Claude's brand
 * mark (`#D97757`) reads fine on any background, so it's a single icon; Copilot's is a near-black
 * `#141413` mark meant for a light surface, invisible against a dark theme, so it needs a separate
 * white-filled variant (`copilot-mark-dark.svg`) selected via the `{ light, dark }` icon form VS
 * Code already supports on both `TreeItem.iconPath` and `TerminalOptions.iconPath`.
 */
export function agentIconPath(extensionUri: vscode.Uri, agent: AgentType): vscode.Uri | { light: vscode.Uri; dark: vscode.Uri } {
  if (agent === 'claude') {
    return vscode.Uri.joinPath(extensionUri, 'resources', 'claude-mark.svg');
  }
  return {
    light: vscode.Uri.joinPath(extensionUri, 'resources', 'copilot-mark.svg'),
    dark: vscode.Uri.joinPath(extensionUri, 'resources', 'copilot-mark-dark.svg'),
  };
}
