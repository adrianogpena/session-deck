/**
 * `@types/node` `^20.11.0` (installed) predates `node:sqlite`'s official type declarations (added
 * around `@types/node` 22.5+). Rather than bump the whole `@types/node` major version for one module,
 * this declares just the surface `discovery/copilotStorage.ts` actually uses. Confirmed working at
 * runtime inside the real extension host (Electron's bundled Node, not just a plain `node` CLI test) —
 * see the commit this file was added in.
 */
declare module 'node:sqlite' {
  export interface DatabaseSyncOptions {
    readOnly?: boolean;
  }

  export class StatementSync {
    get(...params: unknown[]): Record<string, unknown> | undefined;
    all(...params: unknown[]): Record<string, unknown>[];
  }

  export class DatabaseSync {
    constructor(path: string, options?: DatabaseSyncOptions);
    prepare(sql: string): StatementSync;
    close(): void;
  }
}
