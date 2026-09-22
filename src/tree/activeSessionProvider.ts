import * as vscode from 'vscode';
import { SessionTreeProvider, SessionWithProject } from './sessionProvider';
import { readEffectiveSessionStatus } from '../status/sessionStatus';
import { sessionStatusUri } from '../status/sessionStatusDecorationProvider';

/**
 * A compact companion view for the built-in Explorer sidebar — the reference
 * extension (`ShahadIshraq/claude-session-vs-code-extension`) does the same
 * thing, contributing into `views.explorer` alongside its own dedicated
 * container. Deliberately shows only sessions currently "active" (running, or
 * waiting for input) rather than the full project/session tree — Explorer
 * already shares space with a file tree, and Session Deck's own view is where
 * the full picture belongs; this stays to "what needs my attention right
 * now", typically 0 or 1 rows, occasionally a few if several sessions are
 * running at once.
 */
export class ActiveSessionProvider implements vscode.TreeDataProvider<SessionWithProject> {
  private readonly _onDidChangeTreeData = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(private readonly sessionTree: SessionTreeProvider, private readonly extensionUri: vscode.Uri) {}

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getTreeItem(element: SessionWithProject): vscode.TreeItem {
    const { session, projectDisplayName } = element;
    const status = readEffectiveSessionStatus(session.sessionId);

    const item = new vscode.TreeItem(session.displayName, vscode.TreeItemCollapsibleState.None);
    item.description = `${projectDisplayName} · ${status?.status === 'running' ? 'running' : 'waiting for input'}`;
    item.tooltip = `${session.cwd}\n${session.sessionId}`;
    item.iconPath = vscode.Uri.joinPath(this.extensionUri, 'resources', 'claude-mark.svg');
    // Same decoration-based coloring as the main tree — see sessionStatusDecorationProvider.ts.
    item.resourceUri = sessionStatusUri(session.sessionId);
    item.contextValue = 'sessionDeckActiveSession';
    item.command = {
      command: 'sessionDeck.openSession',
      title: 'Open Claude Session',
      arguments: [session],
    };
    return item;
  }

  async getChildren(element?: SessionWithProject): Promise<SessionWithProject[]> {
    if (element) {
      return [];
    }
    const all = await this.sessionTree.listAllSessions();
    return all.filter((entry) => {
      const status = readEffectiveSessionStatus(entry.session.sessionId);
      return status?.status === 'running' || status?.status === 'waiting';
    });
  }
}
