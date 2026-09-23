/** Hand-written types for `node:sqlite` — the installed `@types/node` predates its official declarations. Covers only the surface `copilotStorage.ts` uses. */
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
