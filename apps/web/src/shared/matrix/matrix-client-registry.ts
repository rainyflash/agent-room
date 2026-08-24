import type { MatrixClient } from 'matrix-js-sdk';

export type MatrixClientSource = {
  current(): MatrixClient | null;
  subscribe(listener: () => void): () => void;
};

export class MatrixClientRegistry implements MatrixClientSource {
  readonly #listeners = new Set<() => void>();
  #client: MatrixClient | null = null;

  current(): MatrixClient | null {
    return this.#client;
  }

  replace(client: MatrixClient | null): void {
    if (this.#client === client) {
      return;
    }
    this.#client = client;
    this.#notify();
  }

  refresh(client: MatrixClient): void {
    if (this.#client !== client) {
      return;
    }
    this.#notify();
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  #notify(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}
