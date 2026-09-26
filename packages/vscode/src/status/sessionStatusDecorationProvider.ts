import * as vscode from 'vscode';
import { readEffectiveSessionStatus } from '@session-deck/core';

/** URI scheme used purely to give session tree items a `resourceUri` this provider can key decorations off of. */
export const SESSION_STATUS_DECORATION_SCHEME = 'session-deck-status';

export function sessionStatusUri(sessionId: string): vscode.Uri {
  return vscode.Uri.parse(`${SESSION_STATUS_DECORATION_SCHEME}:/${sessionId}`);
}

/** Uses `vscode.FileDecoration` instead of `TreeItem.iconPath` — a colored icon washes out to the row's plain foreground color when selected, file decorations don't. */
export class SessionStatusDecorationProvider implements vscode.FileDecorationProvider {
  private readonly _onDidChangeFileDecorations = new vscode.EventEmitter<vscode.Uri | vscode.Uri[] | undefined>();
  readonly onDidChangeFileDecorations = this._onDidChangeFileDecorations.event;

  /** Called whenever the underlying status files may have changed, so decorations get re-queried. */
  refresh(): void {
    this._onDidChangeFileDecorations.fire(undefined);
  }

  provideFileDecoration(uri: vscode.Uri): vscode.FileDecoration | undefined {
    if (uri.scheme !== SESSION_STATUS_DECORATION_SCHEME) {
      return undefined;
    }
    const sessionId = uri.path.replace(/^\//, '');
    const status = readEffectiveSessionStatus(sessionId);

    switch (status?.status) {
      case 'running':
        return { badge: '●', color: new vscode.ThemeColor('charts.red'), tooltip: 'Running' };
      case 'waiting':
        return { badge: '●', color: new vscode.ThemeColor('charts.yellow'), tooltip: 'Waiting for input' };
      case 'done':
        return { badge: '●', color: new vscode.ThemeColor('charts.green'), tooltip: 'Done' };
      case 'error':
        // A distinct glyph, not just a red dot — otherwise indistinguishable from "running" at a glance.
        return { badge: '✗', color: new vscode.ThemeColor('errorForeground'), tooltip: 'Exited with an error' };
      default:
        return undefined;
    }
  }
}
